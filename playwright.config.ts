import { defineConfig } from '@playwright/test';
export default defineConfig({
 testDir:'tests',fullyParallel:false,workers:1,timeout:45_000,retries:process.env.CI?1:0,
 use:{baseURL:'http://127.0.0.1:1421',trace:'retain-on-failure',screenshot:'only-on-failure'},
 reporter:process.env.CI?[['github'],['html',{open:'never'}]]:[['list']],
 webServer:{command:'node scripts/test-server.mjs',url:'http://127.0.0.1:1421/health',reuseExistingServer:false,timeout:30_000},
});
