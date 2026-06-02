import * as path from 'path';
import Mocha from 'mocha';
import { glob } from 'glob';

export function run(): Promise<void> {
    const mocha = new Mocha({
        ui: 'bdd',
        color: true,
    });

    const testsRoot = path.resolve(__dirname);

    return new Promise((resolve, reject) => {
        glob('**/**.test.js', { cwd: testsRoot }).then(testFiles => {
            testFiles.forEach(f => mocha.addFile(path.resolve(testsRoot, f)));

            try {
                mocha.run((failures: number) => {
                    if (failures > 0) {
                        reject(new Error(`${failures} tests failed.`));
                    } else {
                        resolve();
                    }
                });
            } catch (err) {
                reject(err);
            }
        }).catch(reject);
    });
}

// Allow running directly
run().then(() => {
    console.log('All tests passed!');
    process.exit(0);
}).catch(err => {
    console.error(err);
    process.exit(1);
});
