import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decodeMembers, decodeWorkspaces, decodeMembership, decodeTeamProjects, teamRequest } from '../src/lib/teams.ts';

const membership = { workspace_id: 'w', user_id: 'u', role: 'reviewer', active: true, revision: 1 };
const workspace = { id: 'w', name: 'fixture', created_at: 1000, membership };

test('team decoders retain explicit roles and revisions', () => {
  assert.deepEqual(decodeMembership(membership), membership);
  assert.deepEqual(decodeWorkspaces([workspace], 'u'), [workspace]);
  assert.deepEqual(decodeMembers([membership], 'w'), [membership]);
});

test('workspace catalog rejects other users, inactive memberships and duplicate ids', () => {
  assert.throws(() => decodeWorkspaces([workspace], 'other'));
  assert.throws(() => decodeWorkspaces([{ ...workspace, membership: { ...membership, active: false } }], 'u'));
  assert.throws(() => decodeWorkspaces([workspace, workspace], 'u'));
});

test('roster rejects mismatched scopes, unknown roles and invalid revisions', () => {
  assert.throws(() => decodeMembers([membership], 'other'));
  assert.throws(() => decodeMembers([membership, membership], 'w'));
  assert.throws(() => decodeMembership({ ...membership, role: 'superuser' }));
  assert.throws(() => decodeMembership({ ...membership, revision: 0 }));
});

test('team requests cannot be redirected to external hosts', async () => {
  await assert.rejects(teamRequest('https://evil.example/v1/web/workspaces'));
  await assert.rejects(teamRequest('//evil.example/v1/web/workspaces'));
  await assert.rejects(teamRequest('/v1/web/workspacesevil'));
  await assert.rejects(teamRequest('/v1/web/workspaces/../../pair'));
  await assert.rejects(teamRequest('/v1/web/workspaces/%2e%2e/pair'));
});

test('logical project decoders bind authority to this user, workspace and project', () => {
  const project = { id: 'p', workspace_id: 'w', name: 'logical', created_at: 1000,
    authority: { workspace_id: 'w', project_id: 'p', user_id: 'u', membership_revision: 1, grant_revision: 2 } };
  assert.deepEqual(decodeTeamProjects([project], 'w', 'u'), [project]);
  assert.throws(() => decodeTeamProjects([project], 'other', 'u'));
  assert.throws(() => decodeTeamProjects([project], 'w', 'other'));
  assert.throws(() => decodeTeamProjects([project, project], 'w', 'u'));
  assert.throws(() => decodeTeamProjects([{ ...project, authority: { ...project.authority, project_id: 'other' } }], 'w', 'u'));
  assert.throws(() => decodeTeamProjects([{ ...project, authority: { ...project.authority, grant_revision: 0 } }], 'w', 'u'));
});
