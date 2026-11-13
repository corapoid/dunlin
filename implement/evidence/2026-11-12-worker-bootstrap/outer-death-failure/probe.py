#!/usr/bin/env python3
"""Tiny real-kernel probe; no provider, credential, source or host mounts.
Run only in the explicitly provisioned named runtime sandbox, never host Python.
"""
import array
import ctypes
import hashlib
import json
import os
import select
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

BWRAP = '/usr/bin/bwrap'
def pdeath():
    parent = os.getppid()
    if ctypes.CDLL(None, use_errno=True).prctl(1, signal.SIGKILL, 0, 0, 0) != 0 or os.getppid() != parent:
        os._exit(123)
    os.setsid()

def wait_until(predicate, seconds=3):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = predicate()
        if value:
            return value
        time.sleep(.01)
    raise AssertionError('bounded observation timed out')

def bootstrap(root):
    info_r, info_w = os.pipe2(os.O_CLOEXEC)
    gate_r, gate_w = os.pipe2(os.O_CLOEXEC)
    os.set_blocking(gate_w, False)
    filled = 0
    while True:
        try:
            filled += os.write(gate_w, b'x' * 4096)
        except BlockingIOError:
            break
    os.set_blocking(gate_w, True)
    script = "import os,time; os.fork(); f=open('ticks','ab',buffering=0);\nwhile True: f.write(b'x'); time.sleep(.02)"
    args = [BWRAP, '--die-with-parent', '--new-session', '--unshare-all', '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/tmp']
    for item in ['/usr', '/bin', '/lib', '/lib64']:
        if Path(item).exists():
            args += ['--ro-bind', item, item]
    args += ['--bind', root, root, '--chdir', root, '--info-fd', str(info_w), '--json-status-fd', str(gate_w), '--', '/usr/bin/python3', '-c', script]
    child = subprocess.Popen(args, pass_fds=(info_w, gate_w), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, preexec_fn=pdeath, env={'PATH': '/usr/bin:/bin'})
    os.close(info_w)
    os.close(gate_w)
    os.set_blocking(info_r, False)
    raw = bytearray()
    deadline = time.monotonic() + 3
    try:
        while time.monotonic() < deadline:
            if not select.select([info_r], [], [], .05)[0]:
                continue
            part = os.read(info_r, 4096)
            if not part:
                break
            raw.extend(part)
            assert len(raw) <= 4096
        else:
            raise AssertionError('info handshake timeout')
        pid = json.loads(raw)['child-pid']
        # Candidate pidfd is NEVER signalled before the owned outer leader is
        # proven still alive/unreaped, with its full status pipe still blocking.
        pidfd = os.pidfd_open(pid)
        assert os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is None
        status = Path(f'/proc/{pid}/status').read_text()
        assert [int(v) for line in status.splitlines() if line.startswith('NSpid:') for v in line.split()[1:]][-1] == 1
        assert os.readlink(f'/proc/{pid}/ns/pid') != os.readlink('/proc/self/ns/pid')
        assert not Path(root, 'ticks').exists(), 'payload ran before pidfd ownership handshake'
        return child, pidfd, gate_r, filled
    finally:
        os.close(info_r)

def release(gate_r, filled):
    while filled:
        part = os.read(gate_r, min(4096, filled))
        assert part == b'x' * len(part)
        filled -= len(part)

def exited(pidfd):
    assert select.select([pidfd], [], [], 3)[0], 'namespace init did not exit'

def helper(sock_fd, root):
    control = socket.socket(fileno=sock_fd)
    child, pidfd, gate_r, filled = bootstrap(root)
    control.sendmsg([b'owned-init'], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array('i', [pidfd]))])
    if control.recv(1) == b'R':
        release(gate_r, filled)
        control.send(b'released')
    # The parent test SIGKILLs this actual supervisor; no in-process watchdog
    # or orderly Python finally block can perform its cleanup.
    while True:
        signal.pause()

if len(sys.argv) > 1 and sys.argv[1] == 'helper':
    helper(int(sys.argv[2]), sys.argv[3])
else:
    print(json.dumps({'version': subprocess.check_output([BWRAP, '--version'], text=True).strip(), 'sha256': hashlib.sha256(Path(BWRAP).read_bytes()).hexdigest()}), flush=True)
    with tempfile.TemporaryDirectory() as directory:
        root = str(Path(directory, 'normal'))
        Path(root).mkdir()
        child, pidfd, gate_r, filled = bootstrap(root)
        release(gate_r, filled)
        wait_until(lambda: Path(root, 'ticks').exists())
        signal.pidfd_send_signal(pidfd, signal.SIGKILL)
        os.killpg(child.pid, signal.SIGKILL)
        exited(pidfd)
        child.wait(timeout=3)
        assert select.select([child.stdout], [], [], 3)[0]
        assert child.stdout.read() == b'', 'silent descendant retained stdout after namespace stop'
        size = Path(root, 'ticks').stat().st_size
        time.sleep(.15)
        assert Path(root, 'ticks').stat().st_size == size
        os.close(pidfd)
        os.close(gate_r)
        # Copy only this trusted probe code into disposable container scratch.
        script = Path(directory, 'probe.py')
        script.write_bytes(Path(__file__).read_bytes())
        for running in [False, True]:
            root = str(Path(directory, 'crash-running' if running else 'crash-startup'))
            Path(root).mkdir()
            owner, peer = socket.socketpair()
            supervisor = subprocess.Popen(['/usr/bin/python3', str(script), 'helper', str(peer.fileno()), root], pass_fds=(peer.fileno(),), env={'PATH': '/usr/bin:/bin'})
            peer.close()
            owner.settimeout(5)
            _, ancillary, _, _ = owner.recvmsg(32, socket.CMSG_SPACE(array.array('i').itemsize))
            fds = array.array('i')
            for level, kind, data in ancillary:
                assert (level, kind) == (socket.SOL_SOCKET, socket.SCM_RIGHTS)
                fds.frombytes(data[:len(data) - len(data) % fds.itemsize])
            assert len(fds) == 1
            initfd = fds[0]
            if running:
                owner.send(b'R')
                assert owner.recv(32) == b'released'
                wait_until(lambda: Path(root, 'ticks').exists())
            supervisor.kill()
            supervisor.wait(timeout=3)
            if not select.select([initfd], [], [], 3)[0]:
                print(json.dumps({'failed_case': 'running' if running else 'startup', 'ticks_exist_after_supervisor_sigkill': Path(root, 'ticks').exists()}), flush=True)
                raise AssertionError('namespace init survived supervisor SIGKILL')
            if running:
                size = Path(root, 'ticks').stat().st_size
                time.sleep(.15)
                assert Path(root, 'ticks').stat().st_size == size
            else:
                assert not Path(root, 'ticks').exists()
            os.close(initfd)
            owner.close()
    print('PASS: blocked bootstrap ownership, namespace pidfd stop, silent-FD descendant, supervisor SIGKILL before/after release', flush=True)
