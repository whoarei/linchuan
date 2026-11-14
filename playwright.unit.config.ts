import { defineConfig } from '@playwright/test';
export default defineConfig({ testDir: 'tests', testMatch: ['request.spec.ts', 'compat.spec.ts'], workers: 1 });
