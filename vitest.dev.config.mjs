import { defineConfig } from 'vitest/config';

export default defineConfig({
    test: {
        name: 'deployed-site',
        include: ['tests/dev/*.test.js'],
        environment: 'node',
        fileParallelism: false,
        maxWorkers: 1,
        testTimeout: 60_000,
        hookTimeout: 30_000,
        expect: { poll: { timeout: 15_000 } },
        watch: false,
    },
});
