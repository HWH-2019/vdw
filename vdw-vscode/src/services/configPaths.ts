import * as fs from 'fs';
import * as path from 'path';

export const CONFIG_FILENAMES = ['.vdwrc', '.vdwrc.yml'] as const;

/** Resolve the configuration file using the same precedence as the CLI. */
export function findConfigPath(workspacePath: string): string | null {
    for (const filename of CONFIG_FILENAMES) {
        const candidate = path.join(workspacePath, filename);
        if (fs.existsSync(candidate)) {
            return candidate;
        }
    }
    return null;
}
