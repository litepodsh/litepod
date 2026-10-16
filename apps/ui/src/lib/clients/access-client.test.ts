import { describe, expect, it } from 'vitest';
import { groupCatalog, inheritedAssignments, type Assignment } from './access-client';

const assignment = (scope_type: Assignment['scope_type'], scope_id: string): Assignment => ({
	id: `${scope_type}-${scope_id}`,
	principal_type: 'user',
	principal_id: 'u',
	principal_name: 'u',
	role_id: '2',
	role_name: 'Editor',
	scope_type,
	scope_id,
	scope_name: scope_id,
	project_id: null,
});

describe('groupCatalog', () => {
	it('groups by the first segment and keeps catalog order', () => {
		const groups = groupCatalog([
			{ name: 'org.read', org_only: true },
			{ name: 'app.read', org_only: false },
			{ name: 'app.env.read', org_only: false },
		]);
		expect(groups.map(([resource]) => resource)).toEqual(['org', 'app']);
		expect(groups[1][1].map((e) => e.name)).toEqual(['app.read', 'app.env.read']);
	});
});

describe('inheritedAssignments', () => {
	const all = [assignment('org', 'o'), assignment('project', 'p1'), assignment('project', 'p2')];

	it('an org scope inherits nothing', () => {
		expect(inheritedAssignments(all, { scopeType: 'org', scopeId: 'o' })).toEqual([]);
	});

	it('a project inherits org-wide assignments only', () => {
		expect(
			inheritedAssignments(all, { scopeType: 'project', scopeId: 'p1' }).map((a) => a.id),
		).toEqual(['org-o']);
	});

	it('an environment also inherits its own project, not siblings', () => {
		expect(
			inheritedAssignments(all, { scopeType: 'environment', scopeId: 'e', projectId: 'p1' }).map(
				(a) => a.id,
			),
		).toEqual(['org-o', 'project-p1']);
	});
});
