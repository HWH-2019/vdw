import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';
import { VDWConfig, Dependency } from '../types/vdwTypes';
import { log, logError } from '../utils/logger';
import { findConfigPath } from './configPaths';

export class ConfigService {
    private config: VDWConfig | null = null;
    private configPath: string | null = null;
    private workspacePath: string;

    constructor(workspacePath: string) {
        this.workspacePath = workspacePath;
    }

    /** Load .vdwrc or .vdwrc.yml from workspace */
    loadConfig(): VDWConfig | null {
        const vdwrcPath = findConfigPath(this.workspacePath);

        if (!vdwrcPath) {
            this.config = null;
            this.configPath = null;
            return null;
        }

        this.config = null;
        this.configPath = vdwrcPath;
        const configName = path.basename(vdwrcPath);

        try {
            const content = fs.readFileSync(vdwrcPath, 'utf-8');
            const parsed = yaml.load(content) as VDWConfig;

            // Basic validation
            if (!parsed || !parsed.workspace || !Array.isArray(parsed.dependencies)) {
                logError(`Invalid ${configName} format`);
                return null;
            }

            // Ensure defaults & backward compatibility
            parsed.dependencies = parsed.dependencies.map((dep: any) => {
                // Backward compatibility: map old 'repo' field
                let remote = dep.remote ?? '';
                let local = dep.local ?? '';
                if (!remote && !local && dep.repo) {
                    if (dep.repo.startsWith('git@') || dep.repo.startsWith('https://') || dep.repo.startsWith('http://')) {
                        remote = dep.repo;
                    } else {
                        local = dep.repo;
                    }
                }
                const { repo, ...rest } = dep;
                return {
                    ...rest,
                    remote,
                    origin: dep.origin ?? '',
                    local,
                    editable: dep.editable ?? false,
                    scope: {
                        include: dep.scope?.include ?? [],
                        exclude: dep.scope?.exclude ?? [],
                    },
                };
            });

            this.config = parsed;
            log(`Loaded config: ${parsed.workspace.name} (${parsed.dependencies.length} deps)`);
            return parsed;
        } catch (err) {
            logError(`Failed to parse ${configName}`, err);
            return null;
        }
    }

    /** Get cached config (reload if null) */
    getConfig(): VDWConfig | null {
        if (!this.config) {
            return this.loadConfig();
        }
        return this.config;
    }

    /** Get the config file path */
    getConfigPath(): string | null {
        return this.configPath;
    }

    /** Check if a supported VDW configuration file exists */
    hasConfig(): boolean {
        return findConfigPath(this.workspacePath) !== null;
    }

    /** Force reload */
    reload(): VDWConfig | null {
        this.config = null;
        return this.loadConfig();
    }

    /** Get dependency count */
    getDependencyCount(): number {
        return this.config?.dependencies.length ?? 0;
    }

    /** Get all dependencies */
    getDependencies(): Dependency[] {
        return this.config?.dependencies ?? [];
    }

    /** Update workspace path */
    setWorkspacePath(newPath: string): void {
        this.workspacePath = newPath;
        this.config = null;
        this.configPath = null;
    }
}
