import * as assert from 'assert';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as yaml from 'js-yaml';
import { minimatch } from 'minimatch';
import { formatLifecycleReport, LifecycleOperation } from '../utils/lifecycle';

describe('Lifecycle result reporting', () => {
    it('keeps archived, skipped and failed workspaces in the same complete report', () => {
        const op: LifecycleOperation = {
            operation_id: 'op-test', kind: 'archive', state: 'partial_failure',
            summary: { scope_total: 3, archived_count: 1, restored_count: 0, not_archived_count: 2, skipped_count: 1, failed_count: 1, unfinished_count: 0 },
            results: [
                { name: 'done', workspace_id: 'ws-done', outcome: 'archived', reason_codes: [], message: '', archive_path: '/archives/done', repositories: [] },
                { name: 'dirty', workspace_id: 'ws-dirty', outcome: 'skipped', reason_codes: ['dirty'], message: '', repositories: [{ path: 'packages/api', dirty: { staged: 1, unstaged: 2, conflicts: 0, untracked: 3, unknown_ignored: 0 } }] },
                { name: 'failed', workspace_id: 'ws-failed', outcome: 'failed', reason_codes: [], message: 'disk full', repositories: [] },
            ],
            progress: { phase: '', archives_done: 0, archives_total: 0, bytes_copied: 0, bytes_total: 0, bytes_verified: 0 },
        };
        const report = formatLifecycleReport(op);
        assert.ok(report.includes('已归档 1 个；未归档 2 个，其中跳过 1 个'));
        assert.ok(report.includes('done [ws-done]'));
        assert.ok(report.includes('dirty [ws-dirty]：未归档（跳过）'));
        assert.ok(report.includes('packages/api：暂存 1，未暂存 2'));
        assert.ok(report.includes('disk full'));
    });
});

// Test VDW types
import {
    VDWConfig,
    Dependency,
    RepositoryTopologyStatus,
    RepositoryRuntimeStatus,
    Scope,
} from '../types/vdwTypes';
import { findConfigPath } from '../services/configPaths';
import {
    buildConflictListArgs,
    buildConflictResolveArgs,
    formatConflictReason,
} from '../utils/linkConflicts';
import {
    dependencyMatchesIdentifier,
    getEditableWorktreePaths,
} from '../utils/gitRepositories';
import {
    collectRepositoryRuntimeStatus,
    formatRepositoryDescription,
    getDependencyName,
    isDefaultScope,
    repositoryStatusPriority,
    summarizeWorkspaceStatus,
} from '../utils/workspaceStatus';

describe('VDW Extension Tests', () => {
    describe('Type Definitions', () => {
        it('should define VDWConfig interface correctly', () => {
            const config: VDWConfig = {
                workspace: { name: 'test', version: '1.0' },
                dependencies: [],
            };
            assert.strictEqual(config.workspace.name, 'test');
            assert.strictEqual(config.dependencies.length, 0);
        });

        it('should define Dependency with all fields', () => {
            const dep: Dependency = {
                remote: 'https://github.com/org/repo.git',
                origin: '',
                local: '',
                path: './libs/repo',
                branch: 'main',
                scope: { include: ['src/**'], exclude: ['**/*.test.ts'] },
                editable: true,
            };
            assert.strictEqual(dep.remote, 'https://github.com/org/repo.git');
            assert.strictEqual(dep.editable, true);
            assert.strictEqual(dep.scope.include[0], 'src/**');
        });
    });

    describe('Config Parsing', () => {
        it('should parse sample .vdwrc YAML', () => {
            const yamlContent = `
workspace:
  name: test-workspace
  version: "1.0"
dependencies:
  - remote: https://github.com/example/frontend.git
    local: ''
    path: ./apps/web
    branch: main
    scope:
      include:
        - "src/components/**"
        - "src/hooks/**"
      exclude:
        - "**/*.test.ts"
    editable: true
  - remote: https://github.com/example/shared.git
    local: ''
    path: ./packages/shared
    tag: v1.0.0
    scope:
      include:
        - "src/**"
    editable: false
`;
            const parsed = yaml.load(yamlContent) as VDWConfig;
            assert.strictEqual(parsed.workspace.name, 'test-workspace');
            assert.strictEqual(parsed.dependencies.length, 2);
            assert.strictEqual(parsed.dependencies[0].branch, 'main');
            assert.strictEqual(parsed.dependencies[0].editable, true);
            assert.strictEqual(parsed.dependencies[1].tag, 'v1.0.0');
            assert.strictEqual(parsed.dependencies[1].editable, false);
        });

        it('should handle empty dependencies', () => {
            const yamlContent = `
workspace:
  name: empty
  version: "1.0"
dependencies: []
`;
            const parsed = yaml.load(yamlContent) as VDWConfig;
            assert.strictEqual(parsed.dependencies.length, 0);
        });
    });

    describe('Config path resolution', () => {
        let workspacePath: string;

        beforeEach(() => {
            workspacePath = fs.mkdtempSync(path.join(os.tmpdir(), 'vdw-vscode-test-'));
        });

        afterEach(() => {
            fs.rmSync(workspacePath, { recursive: true, force: true });
        });

        it('should resolve .vdwrc.yml when .vdwrc is absent', () => {
            fs.writeFileSync(path.join(workspacePath, '.vdwrc.yml'), `
workspace:
  name: yml-workspace
  version: "1.0"
dependencies: []
`);

            assert.strictEqual(findConfigPath(workspacePath), path.join(workspacePath, '.vdwrc.yml'));
        });

        it('should prefer .vdwrc when both config filenames exist', () => {
            fs.writeFileSync(path.join(workspacePath, '.vdwrc'), 'dependencies: []');
            fs.writeFileSync(path.join(workspacePath, '.vdwrc.yml'), 'dependencies: []');

            assert.strictEqual(findConfigPath(workspacePath), path.join(workspacePath, '.vdwrc'));
        });
    });

    describe('Scope Matching', () => {
        it('should match files with minimatch patterns', () => {
            assert.strictEqual(minimatch('src/components/Button.tsx', 'src/components/**'), true);
            assert.strictEqual(minimatch('src/hooks/useAuth.ts', 'src/hooks/**'), true);
            assert.strictEqual(minimatch('src/utils/helper.ts', 'src/components/**'), false);
        });

        it('should respect exclude patterns', () => {
            const file = 'src/components/Button.test.ts';
            const includeMatch = minimatch(file, 'src/components/**');
            const excludeMatch = minimatch(file, '**/*.test.ts');

            assert.strictEqual(includeMatch, true);
            assert.strictEqual(excludeMatch, true);
            // Exclude should take priority: if both match, file is excluded
            const inScope = includeMatch && !excludeMatch;
            assert.strictEqual(inScope, false);
        });

        it('should handle ** recursive patterns', () => {
            assert.strictEqual(minimatch('a/b/c/d.ts', '**/*.ts'), true);
            assert.strictEqual(minimatch('deep/nested/file.rs', 'src/**'), false);
            assert.strictEqual(minimatch('src/deep/nested/file.rs', 'src/**'), true);
        });
    });

    describe('Link conflict handling', () => {
        it('should preserve repository filtering in conflict list arguments', () => {
            assert.deepStrictEqual(
                buildConflictListArgs('repo'),
                ['list', '--json', '--repo', 'repo'],
            );
        });

        it('should pass selected paths when accepting the workspace version', () => {
            assert.deepStrictEqual(
                buildConflictResolveArgs(
                    'workspace',
                    'repo',
                    ['packages/repo/file.ts'],
                ),
                [
                    'resolve', '--accept', 'workspace', '--json',
                    '--repo', 'repo',
                    '--path', 'packages/repo/file.ts',
                ],
            );
        });

        it('should present content differences as file comparison conflicts', () => {
            assert.strictEqual(formatConflictReason('content_differs'), '文件内容不同');
        });

        it('should present missing workspace files as topology conflicts', () => {
            assert.strictEqual(formatConflictReason('target_missing'), '工作区文件缺失');
        });
    });

    describe('Source Control native workspace paths', () => {
        const workspaceRoot = '/Users/test/.vdw/workspaces/example';
        const config: VDWConfig = {
            workspace: { name: 'vdw-workspace', version: '1.0' },
            dependencies: [
                {
                    remote: 'git@example.com:org/repo.git',
                    origin: '',
                    local: '/Users/test/.vdw/store/objects/repo/worktrees/main',
                    path: './packages/repo',
                    branch: 'main',
                    scope: { include: ['src/**'], exclude: [] },
                    editable: true,
                },
                {
                    remote: 'git@example.com:org/readonly.git',
                    origin: '',
                    local: '/Users/test/.vdw/store/objects/readonly/checkout',
                    path: './packages/readonly',
                    branch: 'main',
                    scope: { include: ['**/*'], exclude: [] },
                    editable: false,
                },
            ],
        };

        it('should use the dependency workspace path instead of the legacy Store path', () => {
            assert.deepStrictEqual(
                getEditableWorktreePaths(config, workspaceRoot),
                ['/Users/test/.vdw/workspaces/example/packages/repo'],
            );
        });

        it('should filter a worktree by the canonical repository identifier', () => {
            assert.deepStrictEqual(
                getEditableWorktreePaths(
                    config,
                    workspaceRoot,
                    'git@example.com:org/repo.git',
                ),
                ['/Users/test/.vdw/workspaces/example/packages/repo'],
            );
            assert.strictEqual(
                dependencyMatchesIdentifier(config.dependencies[0], './packages/repo'),
                true,
            );
        });

        it('should derive the target before the worktree is linked', () => {
            const unlinked = {
                ...config,
                dependencies: [{ ...config.dependencies[0], local: '' }],
            };
            assert.deepStrictEqual(
                getEditableWorktreePaths(unlinked, workspaceRoot),
                ['/Users/test/.vdw/workspaces/example/packages/repo'],
            );
        });
    });

    describe('Workspace repository status presentation', () => {
        const dependency: Dependency = {
            remote: 'git@example.com:org/example.git',
            origin: '',
            local: './packages/example',
            path: './packages/example',
            branch: 'feature/workspace',
            scope: { include: ['**/*'], exclude: [] },
            editable: true,
        };

        const readyStatus: RepositoryRuntimeStatus = {
            dependencyPath: dependency.path,
            fullPath: '/workspace/packages/example',
            state: 'ready',
            exists: true,
            scmRegistered: true,
            desiredRevision: 'feature/workspace',
            currentBranch: 'feature/workspace',
            changeCount: 0,
            ahead: 0,
            behind: 0,
            trackedLinkCount: 0,
            conflictCount: 0,
            missingLinkCount: 0,
            detachedEquivalentCount: 0,
        };

        it('should keep the repository name separate from the long branch', () => {
            assert.strictEqual(getDependencyName(dependency), 'example');
            assert.strictEqual(
                formatRepositoryDescription(dependency, readyStatus),
                'feature/workspace',
            );
        });

        it('should omit the default all-files AI scope', () => {
            assert.strictEqual(isDefaultScope(dependency), true);
            assert.strictEqual(
                isDefaultScope({
                    ...dependency,
                    scope: { include: ['src/**'], exclude: [] },
                }),
                false,
            );
        });

        it('should show dirty and remote synchronization counts compactly', () => {
            const dirtyStatus: RepositoryRuntimeStatus = {
                ...readyStatus,
                state: 'dirty',
                changeCount: 12,
                ahead: 1,
                behind: 2,
            };
            assert.strictEqual(
                formatRepositoryDescription(dependency, dirtyStatus),
                'feature/workspace · 12 changes · ↑1 ↓2',
            );

            const driftStatus: RepositoryRuntimeStatus = {
                ...dirtyStatus,
                state: 'branchDrift',
                currentBranch: 'feature/other',
            };
            assert.strictEqual(
                formatRepositoryDescription(dependency, driftStatus),
                'feature/other → feature/workspace · 12 changes · ↑1 ↓2',
            );
            assert.strictEqual(summarizeWorkspaceStatus([driftStatus]).dirtyCount, 1);
        });

        it('should rank actionable issues before dirty and ready repositories', () => {
            const missingStatus: RepositoryRuntimeStatus = {
                ...readyStatus,
                state: 'missing',
                exists: false,
                scmRegistered: false,
            };
            const dirtyStatus: RepositoryRuntimeStatus = {
                ...readyStatus,
                state: 'dirty',
                changeCount: 1,
            };
            assert.ok(repositoryStatusPriority(missingStatus) < repositoryStatusPriority(dirtyStatus));
            assert.ok(repositoryStatusPriority(dirtyStatus) < repositoryStatusPriority(readyStatus));
            assert.deepStrictEqual(
                summarizeWorkspaceStatus([missingStatus, dirtyStatus, readyStatus]),
                {
                    repositories: [missingStatus, dirtyStatus, readyStatus],
                    issueCount: 1,
                    dirtyCount: 1,
                },
            );
        });

        it('should derive runtime states from worktree and Git repository evidence', () => {
            const workspacePath = fs.mkdtempSync(path.join(os.tmpdir(), 'vdw-status-test-'));
            const repositoryPath = path.join(workspacePath, 'packages', 'example');
            try {
                assert.strictEqual(
                    collectRepositoryRuntimeStatus(dependency, workspacePath).state,
                    'missing',
                );

                fs.mkdirSync(repositoryPath, { recursive: true });
                assert.strictEqual(
                    collectRepositoryRuntimeStatus(dependency, workspacePath).state,
                    'invalidWorktree',
                );

                fs.writeFileSync(path.join(repositoryPath, '.git'), 'gitdir: /tmp/example.git');
                assert.strictEqual(
                    collectRepositoryRuntimeStatus(dependency, workspacePath).state,
                    'scmMissing',
                );

                const gitApi = {
                    repositories: [{
                        rootUri: { fsPath: repositoryPath },
                        state: {
                            HEAD: { name: 'feature/other', ahead: 1, behind: 2 },
                            mergeChanges: [],
                            indexChanges: [],
                            workingTreeChanges: [{}],
                            untrackedChanges: [],
                        },
                    }],
                };
                const drift = collectRepositoryRuntimeStatus(dependency, workspacePath, gitApi);
                assert.strictEqual(drift.state, 'branchDrift');
                assert.strictEqual(drift.changeCount, 1);
                assert.strictEqual(drift.ahead, 1);
                assert.strictEqual(drift.behind, 2);

                const dirty = collectRepositoryRuntimeStatus(
                    { ...dependency, branch: 'feature/other' },
                    workspacePath,
                    gitApi,
                );
                assert.strictEqual(dirty.state, 'dirty');
            } finally {
                fs.rmSync(workspacePath, { recursive: true, force: true });
            }
        });

        it('should prioritize CLI topology conflicts over Git working tree state', () => {
            const topology: RepositoryTopologyStatus = {
                repo: dependency.remote,
                path: dependency.path,
                full_path: '/workspace/packages/example',
                editable: false,
                declared_revision: 'feature/workspace',
                topology_state: 'conflict',
                recommended_action: 'resolve_conflicts',
                tracked_link_count: 10,
                conflict_count: 2,
                missing_count: 0,
                detached_equivalent_count: 0,
                message: '2 tracked targets differ',
            };
            const workspacePath = fs.mkdtempSync(path.join(os.tmpdir(), 'vdw-topology-test-'));
            try {
                fs.mkdirSync(path.join(workspacePath, 'packages', 'example'), { recursive: true });
                const status = collectRepositoryRuntimeStatus(
                    { ...dependency, editable: false },
                    workspacePath,
                    undefined,
                    topology,
                );
                assert.strictEqual(status.state, 'topologyConflict');
                assert.strictEqual(status.conflictCount, 2);
                assert.strictEqual(status.recommendedAction, 'resolve_conflicts');
                assert.strictEqual(status.topologyMessage, '2 tracked targets differ');
            } finally {
                fs.rmSync(workspacePath, { recursive: true, force: true });
            }
        });
    });

    describe('Repository view contributions', () => {
        const packageJson = JSON.parse(
            fs.readFileSync(path.resolve(__dirname, '..', '..', 'package.json'), 'utf-8'),
        );

        it('should contribute VDW repositories to Source Control without a separate activity container', () => {
            const scmViews = packageJson.contributes.views.scm;
            assert.ok(Array.isArray(scmViews));
            assert.strictEqual(scmViews[0].id, 'vdwWorkspace');
            assert.strictEqual(scmViews[0].name, 'VDW Repositories');
            assert.strictEqual(packageJson.contributes.viewsContainers, undefined);
        });

        it('should keep reconcile out of the inline title navigation group', () => {
            const reconcileMenu = packageJson.contributes.menus['view/title'].find(
                (entry: any) => entry.command === 'vdw.relink',
            );
            assert.ok(reconcileMenu);
            assert.notStrictEqual(reconcileMenu.group, 'navigation');
            assert.strictEqual(reconcileMenu.group, '1_workspace@1');
        });

        it('should contribute a repository conflict resolution action', () => {
            const command = packageJson.contributes.commands.find(
                (entry: any) => entry.command === 'vdw.resolveRepoConflicts',
            );
            const menu = packageJson.contributes.menus['view/item/context'].find(
                (entry: any) => entry.command === 'vdw.resolveRepoConflicts',
            );
            assert.ok(command);
            assert.ok(menu);
            assert.ok(menu.when.includes('vdwRepoConflict'));
        });
    });
});
