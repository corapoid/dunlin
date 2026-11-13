#!/usr/bin/env python3
# Execute ONLY in the named offline 128MiB runtime sandbox, never host Python.
import array,ctypes,errno,hashlib,json,os,select,signal,socket,subprocess,sys,tempfile,time
from pathlib import Path
BWRAP='/usr/bin/bwrap'
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

def guardian(fd,root,phase):
    control=socket.socket(fileno=fd)
    child,initfd,gate,filled=bootstrap(root)
    if phase=='before-handoff':
        # We intentionally send no PID and no pidfd to Main in this phase.
        control.send(b'bootstrap-held')
    else:
        control.sendmsg([b'owned-init'],[(socket.SOL_SOCKET,socket.SCM_RIGHTS,array.array('i',[initfd,child.stdout.fileno()]))])
        if phase=='running':
            assert control.recv(1)==b'R'
            release(gate,filled)
            control.send(b'released')
    while True: signal.pause()

def direct_children():
    result=set()
    for path in Path('/proc/self/task').glob('*/children'):
        try: result.update(int(value) for value in path.read_text().split())
        except FileNotFoundError: pass
    return result

def status(pid):
    fields={line.split(':',1)[0]:line.split(':',1)[1].strip() for line in Path(f'/proc/{pid}/status').read_text().splitlines() if ':' in line}
    return fields

def owned_cleanup():
    # No other spawner, SIGCHLD auto-reaper or wait-any thread exists here.
    # All adopted tasks stay alive or zombie until this exact pidfd is reaped.
    end=time.monotonic()+8
    records=[]
    while True:
        children=direct_children()
        assert len(children)<=32
        if not children: return records
        assert time.monotonic()<end,'cleanup failed to settle'
        batch=[]
        for pid in sorted(children):
            try: fd=os.pidfd_open(pid)
            except ProcessLookupError: continue
            try:
                # Ownership check is on the handle AFTER acquisition, not a
                # PID-only check which could precede a recycled-ID lookup.
                os.waitid(os.P_PIDFD,fd,os.WEXITED|os.WNOHANG|os.WNOWAIT)
                fields=status(pid)
                assert int(fields['PPid'])==os.getpid()
                assert all(int(uid)==os.getuid() for uid in fields['Uid'].split())
                nspid=[int(value) for value in fields['NSpid'].split()]
                nested=os.readlink(f'/proc/{pid}/ns/pid')!=os.readlink('/proc/self/ns/pid')
                record={'pid':pid,'ppid':int(fields['PPid']),'uid':os.getuid(),'namespace_init':nested and nspid[-1]==1}
                try: signal.pidfd_send_signal(fd,signal.SIGKILL)
                except ProcessLookupError: pass
                batch.append((fd,record))
            except ChildProcessError:
                os.close(fd) # Never signal a handle not proven to be our child.
                raise AssertionError('enumerated task was reaped/changed outside the cleanup owner')
            except BaseException:
                os.close(fd)
                raise
        for fd,record in batch:
            try:
                remaining=end-time.monotonic()
                assert remaining>0 and select.select([fd],[],[],remaining)[0],'owned task did not exit'
                receipt=os.waitid(os.P_PIDFD,fd,os.WEXITED|os.WNOWAIT)
                assert receipt.si_pid==record['pid']
                os.waitid(os.P_PIDFD,fd,os.WEXITED)
                record['pidfd_exit_observed']=True
                records.append(record)
            finally: os.close(fd)

if len(sys.argv)>1:
    guardian(int(sys.argv[2]),sys.argv[3],sys.argv[1])
else:
    libc=ctypes.CDLL(None,use_errno=True)
    assert libc.prctl(36,1,0,0,0)==0 # PR_SET_CHILD_SUBREAPER
    active=ctypes.c_int(0)
    assert libc.prctl(37,ctypes.byref(active),0,0,0)==0 and active.value==1
    signal.signal(signal.SIGCHLD,signal.SIG_DFL)
    print(json.dumps({'bwrap_sha256':hashlib.sha256(Path(BWRAP).read_bytes()).hexdigest(),'subreaper':True}),flush=True)
    with tempfile.TemporaryDirectory() as directory:
        for phase in ['before-handoff','after-handoff-blocked','running']:
            root=Path(directory,phase);root.mkdir()
            owner,peer=socket.socketpair();owner.settimeout(5)
            child=subprocess.Popen(['/usr/bin/python3','-I',__file__,phase,str(peer.fileno()),str(root)],pass_fds=(peer.fileno(),),env={'HOME':'/tmp','PATH':'/usr/bin:/bin','TMPDIR':'/tmp'})
            peer.close();held=[]
            if phase=='before-handoff': assert owner.recv(32)==b'bootstrap-held'
            else:
                message,ancillary,flags,_=owner.recvmsg(32,socket.CMSG_SPACE(2*array.array('i').itemsize))
                assert message==b'owned-init' and not flags
                for level,kind,data in ancillary:
                    assert (level,kind)==(socket.SOL_SOCKET,socket.SCM_RIGHTS)
                    values=array.array('i');values.frombytes(data[:len(data)-len(data)%values.itemsize]);held.extend(values)
                assert len(held)==2
            if phase=='running':
                owner.send(b'R');assert owner.recv(32)==b'released'
                wait_until(lambda:(root/'ticks').exists() and (root/'ticks').stat().st_size>0)
            else: assert not (root/'ticks').exists(),'payload ran without Main acknowledgment'
            child.kill();child.wait(timeout=3) # Reap only the known guardian.
            adopted=owned_cleanup()
            assert adopted and any(item['namespace_init'] for item in adopted)
            assert not direct_children()
            if held:
                assert select.select([held[0]],[],[],1)[0],'transferred init handle still alive'
                assert select.select([held[1]],[],[],1)[0] and os.read(held[1],65536)==b'','silent descendant retained stdout'
                for fd in held: os.close(fd)
            if phase=='running':
                size=(root/'ticks').stat().st_size
                time.sleep(.15)
                assert (root/'ticks').stat().st_size==size,'held writable FD survived namespace cleanup'
            else: assert not (root/'ticks').exists()
            print(json.dumps({'case':phase,'guardian_sigkill':True,'adopted':adopted,'payload_ran':phase=='running','all_children_reaped':True,'held_fd_stopped':bool(held)}),flush=True)
            owner.close()
    print('PASS dedicated Main subreaper: owned pidfd cleanup before handoff, after blocked handoff, and running silent held-FD descendants',flush=True)
