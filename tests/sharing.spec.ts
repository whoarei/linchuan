import { test, expect, type Browser, type Page } from '@playwright/test';
import { readFile } from 'node:fs/promises';
async function boot(index=0){return JSON.parse(await readFile(`.playwright/bootstrap-${index}.json`,'utf8'));}
async function owner(browser:Browser,index=0){const b=await boot(index);const context=await browser.newContext();await context.addInitScript(token=>sessionStorage.setItem('linchuan-token',token),b.token);const page=await context.newPage();await page.goto(b.address);await expect(page.getByRole('heading',{name:'发送内容'})).toBeVisible();return {page,context,b};}
async function pair(page:Page,name:string){const b=await boot();await page.goto(b.address);await page.getByLabel('你的设备名称').fill(name);await page.getByLabel('六位配对码').fill(b.pairing_code);await page.getByRole('button',{name:'连接设备',exact:true}).click();await expect(page.getByRole('heading',{name:'发送内容'})).toBeVisible();}

test('browser pairs, sends text and a Unicode filename; owner accepts; browser downloads original bytes',async({browser,page})=>{
 const host=await owner(browser);await pair(page,'网页手机');await page.getByRole('button',{name:/测试电脑 Linux/}).click();await page.getByRole('textbox',{name:'发送文字'}).fill('来自浏览器的一段文字');
 const bytes=Buffer.from('邻传原始内容\n图片、文字与文件');await page.locator('input[type=file]').setInputFiles({name:'你好 邻传.txt',mimeType:'text/plain',buffer:bytes});await page.getByRole('button',{name:/发送内容 1/}).click();
 await host.page.getByRole('button',{name:/收件箱/}).first().click();const card=host.page.locator('.transfer-card').filter({hasText:'来自浏览器的一段文字'});await expect(card).toBeVisible();await card.getByRole('button',{name:'接收',exact:true}).click();await expect(card.getByText('已完成',{exact:true})).toBeVisible();
 const senderCard=page.locator('.transfer-card').filter({hasText:'来自浏览器的一段文字'});await expect(senderCard.getByText('已完成',{exact:true})).toBeVisible();const downloaded=page.waitForEvent('download');await senderCard.getByRole('button',{name:'下载 你好 邻传.txt'}).click();const result=await downloaded;expect(result.suggestedFilename()).toBe('你好 邻传.txt');const filePath=await result.path();expect(await readFile(filePath!)).toEqual(bytes);await host.context.close();
});
test('application owner connects another application and sends directly with independent confirmation',async({browser})=>{
 const a=await owner(browser);const b=await owner(browser,1);await a.page.getByRole('button',{name:'通过地址连接'}).click();await a.page.getByLabel('对方设备地址').fill(b.b.address);await a.page.getByRole('button',{name:'连接设备',exact:true}).click();await expect(a.page.getByText('设备已连接',{exact:true})).toBeVisible();await a.page.getByRole('textbox',{name:'发送文字'}).fill('应用之间直接传输');await a.page.getByRole('button',{name:/发送内容 1/}).click();
 await b.page.getByRole('button',{name:/收件箱/}).first().click();const card=b.page.locator('.transfer-card').filter({hasText:'应用之间直接传输'});await card.getByRole('button',{name:'接收',exact:true}).click();await expect(card.getByText('已完成',{exact:true})).toBeVisible();await expect(a.page.locator('.transfer-card').filter({hasText:'应用之间直接传输'}).getByText('已完成',{exact:true})).toBeVisible();await a.context.close();await b.context.close();
});
test('mobile browser layout stays within viewport and can receive owner text',async({browser})=>{
 const context=await browser.newContext({viewport:{width:390,height:844},isMobile:true,hasTouch:true});const page=await context.newPage();await pair(page,'移动网页');expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
 const host=await owner(browser);await host.page.getByRole('button',{name:/移动网页 浏览器/}).click();await host.page.getByRole('textbox',{name:'发送文字'}).fill('发给手机浏览器');await host.page.getByRole('button',{name:/发送内容 1/}).click();await page.locator('.mobile-nav').getByRole('button',{name:/收件箱/}).click();const card=page.locator('.transfer-card').filter({hasText:'发给手机浏览器'});await card.getByRole('button',{name:'接收',exact:true}).click();await expect(card.getByText('已完成',{exact:true})).toBeVisible();await page.screenshot({path:'.playwright/mobile.png',fullPage:true});await host.page.getByRole('button',{name:'发送内容',exact:true}).click();await host.page.screenshot({path:'.playwright/desktop.png',fullPage:true});await host.context.close();await context.close();
});
test('wrong pairing code shows a clear error',async({page})=>{
 const b=await boot();await page.goto(b.address);await page.getByLabel('你的设备名称').fill('错误配对');await page.getByLabel('六位配对码').fill(b.pairing_code==='000000'?'111111':'000000');await page.getByRole('button',{name:'连接设备',exact:true}).click();await expect(page.getByRole('alert')).toBeVisible();await expect(page.getByRole('heading',{name:'连接设备'})).toBeVisible();
});

test('mobile browser can disconnect without desktop controls', async ({ browser }) => {
 const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
 const page = await context.newPage();
 await pair(page, '断开测试');
 await page.locator('.mobile-nav').getByRole('button', { name: '断开当前会话' }).click();
 await expect(page.getByRole('heading', { name: '连接设备', exact: true })).toBeVisible();
 expect(await page.evaluate(() => sessionStorage.getItem('linchuan-token'))).toBeNull();
 await context.close();
});

test('modal traps keyboard focus, closes with Escape, and restores its opener', async ({ browser }) => {
 const host = await owner(browser);
 const opener = host.page.getByRole('button', { name: '通过地址连接', exact: true });
 await opener.click();
 const dialog = host.page.getByRole('dialog', { name: '通过地址连接' });
 await expect(dialog).toBeVisible();
 await expect(dialog.getByLabel('对方设备地址')).toBeFocused();
 await host.page.keyboard.press('Shift+Tab');
 await expect(dialog.getByRole('button', { name: '关闭', exact: true })).toBeFocused();
 await host.page.keyboard.press('Shift+Tab');
 await expect(dialog.getByRole('button', { name: '连接设备', exact: true })).toBeFocused();
 await host.page.keyboard.press('Tab');
 await expect(dialog.getByRole('button', { name: '关闭', exact: true })).toBeFocused();
 await host.page.keyboard.press('Escape');
 await expect(dialog).toBeHidden();
 await expect(opener).toBeFocused();
 await host.context.close();
});

test('UTF-8 limit blocks oversized text before creating a draft or uploading files', async ({ page }) => {
 await pair(page, '文字限制测试');
 await page.getByRole('button', { name: /测试电脑 Linux/ }).click();
 await page.locator('input[type=file]').setInputFiles({ name: 'small.txt', mimeType: 'text/plain', buffer: Buffer.from('small') });
 const drafts: string[] = [];
 page.on('request', request => {
  if (request.url().includes('/api/drafts')) drafts.push(request.url());
 });
 const textarea = page.getByRole('textbox', { name: '发送文字' });
 const send = page.getByRole('button', { name: /发送内容 1/ });
 await textarea.fill('a'.repeat(65_536));
 await expect(send).toBeEnabled();
 await textarea.fill('a'.repeat(65_537));
 await expect(send).toBeDisabled();
 await expect(textarea).toHaveAttribute('aria-invalid', 'true');
 await expect(page.getByRole('alert')).toContainText('文件尚未上传');
 // JS string length is not UTF-8 byte length: each Chinese character is 3 bytes.
 await textarea.fill('中'.repeat(21_845) + 'a');
 await expect(send).toBeEnabled();
 await expect(page.getByRole('alert')).toHaveCount(0);
 await textarea.fill('中'.repeat(21_846));
 await expect(send).toBeDisabled();
 await expect(page.getByRole('alert')).toContainText('65,536');
 expect(drafts).toEqual([]);
 await textarea.fill('已缩短');
 await expect(send).toBeEnabled();
 await expect(textarea).toHaveAttribute('aria-invalid', 'false');
});

test('startup failure offers a compact retry and recovers without reloading', async ({ page }) => {
 const b = await boot();
 let fail = true;
 await page.route('**/api/info', route => fail
  ? route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: '暂时无法连接' }) })
  : route.continue());
 await page.goto(b.address);
 await expect(page.getByRole('alert')).toContainText('暂时无法连接');
 fail = false;
 await page.getByRole('button', { name: '重试', exact: true }).click();
 await expect(page.getByRole('heading', { name: '连接设备', exact: true })).toBeVisible();
 await expect(page.getByRole('alert')).toHaveCount(0);
});

test('native string startup errors show a retry instead of the pairing screen', async ({ page }) => {
 const bootstrap = await boot();
 await page.addInitScript(result => {
  let attempts = 0;
  Object.defineProperties(window, {
   isTauri: { value: true },
   __TAURI_INTERNALS__: {
    value: {
     invoke: (command: string) => {
      if (command !== 'bootstrap') throw new Error('unexpected invoke: ' + command);
      if (++attempts === 1) return Promise.reject('原生服务暂时无法启动');
      return Promise.resolve(result);
     },
    },
   },
  });
 }, bootstrap);
 await page.goto(bootstrap.address);
 await expect(page.getByRole('alert')).toContainText('原生服务暂时无法启动');
 await expect(page.getByRole('heading', { name: '连接设备', exact: true })).toHaveCount(0);
 await page.getByRole('button', { name: '重试', exact: true }).click();
 await expect(page.getByRole('heading', { name: '发送内容', exact: true })).toBeVisible();
 await expect(page.getByRole('alert')).toHaveCount(0);
});

test('initial authenticated state failure is retryable rather than a false pairing screen', async ({ browser }) => {
 const b = await boot();
 const context = await browser.newContext();
 await context.addInitScript(token => sessionStorage.setItem('linchuan-token', token), b.token);
 const page = await context.newPage();
 let fail = true;
 await page.route('**/api/state', route => fail
  ? route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: '设备暂时不可用' }) })
  : route.continue());
 await page.goto(b.address);
 await expect(page.getByRole('alert')).toContainText('设备暂时不可用');
 fail = false;
 await page.getByRole('button', { name: '重试', exact: true }).click();
 await expect(page.getByRole('heading', { name: '发送内容', exact: true })).toBeVisible();
 await context.close();
});

test('stalled startup times out and retry starts a fresh request', async ({ page }) => {
 const b = await boot();
 await page.clock.install();
 await page.route('**/api/info', () => { /* Simulate a peer that never replies. */ });
 const request = page.waitForRequest('**/api/info');
 await page.goto(b.address);
 await request;
 await page.clock.fastForward(15_001);
 await expect(page.getByRole('alert')).toContainText('连接超时');
 await page.unroute('**/api/info');
 await page.getByRole('button', { name: '重试', exact: true }).click();
 await expect(page.getByRole('heading', { name: '连接设备', exact: true })).toBeVisible();
});

test('a stalled refresh releases its lock and a later poll restores connection', async ({ browser }) => {
 const host = await owner(browser);
 await host.page.clock.install();
 await host.page.route('**/api/state', () => {});
 const stalled = host.page.waitForRequest('**/api/state');
 await host.page.clock.fastForward(3_001);
 await stalled;
 await host.page.clock.fastForward(15_001);
 await expect(host.page.getByText('连接中断，正在重试', { exact: true })).toBeVisible();
 await host.page.unroute('**/api/state');
 await host.page.clock.fastForward(3_001);
 await expect(host.page.getByText('局域网已连接', { exact: true })).toBeVisible();
 await host.context.close();
});
