import * as assert from 'assert';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as yaml from 'js-yaml';
import { minimatch } from 'minimatch';

// Test VDW types
import { VDWConfig, Dependency, Scope } from '../types/vdwTypes';
import { findConfigPath } from '../services/configPaths';

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
});
