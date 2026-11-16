import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

async function boot(index=3) {
  return JSON.parse(await readFile(`.playwright/bootstrap-${index}.json`,"utf8"));
}
for(const mobile of [false,true]) {
  test(`browser opens only its paired host on ${mobile?"mobile":"desktop"}`,async({browser})=>{
    const b=await boot();const context=await browser.newContext({viewport:mobile?{width:390,height:844}:{width:1180,height:820}});
    const page=await context.newPage();await page.goto(b.address);
    await page.getByLabel("六位配对码").fill(b.pairing_code);
    await page.getByRole("button",{name:"连接设备",exact:true}).click();
    await expect(page.getByRole("textbox",{name:"发送文字"})).toBeVisible();
    await expect(page.locator(".conversation-panel")).toHaveCount(0);
    await expect(page.getByRole("button",{name:"添加",exact:true})).toHaveCount(0);
    await expect(page.getByRole("button",{name:"新建群聊",exact:true})).toHaveCount(0);
    const state=await page.evaluate(async()=>{
      const response=await fetch("/api/state",{headers:{Authorization:`Bearer ${localStorage.getItem("linchuan-token")}`}});return response.json();
    });
    expect(state.devices).toEqual([]);expect(state.groups).toEqual([]);
    await expect(page.getByRole("heading",{name:state.info.name,exact:true})).toBeVisible();
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`.playwright/host-only-${mobile?"mobile":"desktop"}.png`,fullPage:true});
    await page.getByRole("button",{name:"设置",exact:true}).click();
    await expect(page.getByLabel("设备名称",{exact:true})).toBeVisible();
    await context.close();
  });
}

test("browser confirmation waits for actual download before reporting delivery",async({page,request})=>{
  const b=await boot();await page.goto(b.address);await page.getByLabel("六位配对码").fill(b.pairing_code);
  await page.getByRole("button",{name:"连接设备",exact:true}).click();
  await expect(page.getByRole("textbox",{name:"发送文字"})).toBeVisible();
  const token=await page.evaluate(()=>localStorage.getItem("linchuan-token"));
  const identity=await(await request.get(b.address+"/api/state",{headers:{Authorization:`Bearer ${token}`}})).json();
  const headers={Authorization:`Bearer ${b.token}`};const bytes=Buffer.from("直接下载，不把确认当作已保存");
  const draft=await(await request.post(b.address+"/api/drafts",{headers,data:{files:[{name:"下载验证.txt",size:bytes.length,mime:"text/plain"}]}})).json();
  expect((await request.put(`${b.address}/api/drafts/${draft.id}/files/${draft.files[0].id}`,{headers,data:bytes})).ok()).toBeTruthy();
  const sent=await(await request.post(`${b.address}/api/drafts/${draft.id}/send`,{headers,data:{text:"浏览器下载状态验证",targets:[identity.me.id]}})).json();
  const browserHeaders={Authorization: "Bearer "+token};
  expect((await request.post(b.address+"/api/transfers/"+sent.ids[0]+"/decision",{headers:browserHeaders,data:{accept:true}})).ok()).toBeTruthy();
  let state=await(await request.get(b.address+"/api/state",{headers})).json();
  expect(state.transfers.find((t:any)=>t.id===sent.ids[0]).status).toBe("accepted");
  const downloaded=page.waitForEvent("download");
  await page.locator(".message").filter({hasText:"浏览器下载状态验证"}).getByRole("button",{name:"下载 下载验证.txt",exact:true}).click();
  const download=await downloaded;expect(await readFile((await download.path())!)).toEqual(bytes);
  await expect.poll(async()=>{
    state=await(await request.get(b.address+"/api/state",{headers})).json();return state.transfers.find((t:any)=>t.id===sent.ids[0]).status;
  }).toBe("completed");
});

test("native file picker submits a source reference without uploading a draft copy",async({browser,request})=>{
  const b=await boot();const peer=await boot(0);const headers={Authorization:`Bearer ${b.token}`};
  const target=await(await request.post(b.address+"/api/peers",{headers,data:{address:peer.address}})).json();
  await request.post(b.address+"/api/conversations/direct",{headers,data:{device_id:target.id}});
  const peerInfo=await(await request.get(peer.address+"/api/info")).json();
  const context=await browser.newContext();
  await context.addInitScript((b)=>{
    const scope=window as any;scope.isTauri=true;scope.__TAURI_INTERNALS__={async invoke(command:string){
      if(command==="bootstrap")return b;
      if(command==="desktop_window_controls")return false;
      if(command==="received_files")return [];
      if(command==="choose_send_files")return[{source_id:"native-file-reference",name:"大文件.img",size:7*1024**3,mime:"application/octet-stream"}];
    }};
  },b);
  const page=await context.newPage();let draftBody:any;let uploads=0;
  page.on("request",(r)=>{if(/\/api\/drafts\/[^/]+\/files\//.test(r.url()))uploads++;});
  await page.route("**/api/drafts",async(route)=>{
    draftBody=route.request().postDataJSON();await route.fulfill({json:{id:"native-draft",files:[{id:"native-attachment",name:"大文件.img",size:7*1024**3,mime:"application/octet-stream"}]}});
  });
  await page.route("**/api/drafts/native-draft/send",(route)=>route.fulfill({json:{ids:[]}}));
  await page.goto(b.address);
  await page.getByRole("button",{name:`打开 ${peerInfo.name}`,exact:true}).click();
  await page.getByRole("button",{name:"添加文件或图片",exact:true}).click();
  await expect(page.locator(".composer-files")).toContainText("大文件.img");
  await page.getByRole("button",{name:"发送",exact:true}).click();
  await expect.poll(()=>draftBody?.files?.[0]?.source_id).toBe("native-file-reference");
  expect(draftBody.streaming).toBe(false);expect(uploads).toBe(0);
  await context.close();
});

test("keyboard shortcuts send text while plain Enter inserts a newline", async ({page,request}) => {
  const b = await boot();
  await page.goto(b.address);
  await page.getByLabel("六位配对码").fill(b.pairing_code);
  await page.getByRole("button",{name:"连接设备",exact:true}).click();
  const composer = page.getByRole("textbox",{name:"发送文字"});
  for (const shortcut of ["Control+Enter", "Meta+Enter"]) {
    const text = "键盘发送 " + shortcut;
    await composer.fill(text);
    await composer.press("Enter");
    await expect(composer).toHaveValue(text + "\n");
    await composer.press(shortcut);
    const card = page.locator(".message").filter({hasText:text});
    await expect(card.getByText("已送达",{exact:true})).toBeVisible();
    await expect(card.getByRole("button",{name:"接收",exact:true})).toHaveCount(0);
    await expect(composer).toHaveValue("");
  }
  const state = await(await request.get(b.address+"/api/state",{headers:{Authorization:"Bearer "+b.token}})).json();
  for (const shortcut of ["Control+Enter", "Meta+Enter"]) {
    expect(state.transfers.filter((t:any)=>t.text.trim()==="键盘发送 "+shortcut)).toHaveLength(1);
  }
});

test("browser image uploads need no host confirmation and small received images display automatically", async ({page,request}) => {
  const b = await boot();
  await page.goto(b.address);
  await page.getByLabel("六位配对码").fill(b.pairing_code);
  await page.getByRole("button",{name:"连接设备",exact:true}).click();
  await expect(page.getByRole("textbox",{name:"发送文字"})).toBeVisible();
  const browserToken = await page.evaluate(()=>localStorage.getItem("linchuan-token"));
  const browserHeaders = {Authorization:"Bearer "+browserToken};
  const ownerHeaders = {Authorization:"Bearer "+b.token};
  const state = await(await request.get(b.address+"/api/state",{headers:browserHeaders})).json();
  const bytes = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a2uoAAAAASUVORK5CYII=","base64");
  await page.locator('input[type="file"]').setInputFiles({name:"网页自动接收.png",mimeType:"image/png",buffer:bytes});
  await page.getByRole("textbox",{name:"发送文字"}).fill("自动上传图片");
  await page.getByRole("button",{name:"发送",exact:true}).click();
  await expect(page.locator(".message").filter({hasText:"自动上传图片"}).getByText("已送达",{exact:true})).toBeVisible();
  const draft = await(await request.post(b.address+"/api/drafts",{headers:ownerHeaders,data:{files:[{name:"客户端自动接收.png",size:bytes.length,mime:"image/png"}]}})).json();
  expect((await request.put(`${b.address}/api/drafts/${draft.id}/files/${draft.files[0].id}`,{headers:ownerHeaders,data:bytes})).ok()).toBeTruthy();
  const sent = await(await request.post(`${b.address}/api/drafts/${draft.id}/send`,{headers:ownerHeaders,data:{text:"图片自动展示",targets:[state.me.id]}})).json();
  const card = page.locator(".message").filter({hasText:"图片自动展示"});
  await expect(card.getByRole("button",{name:"接收",exact:true})).toHaveCount(0);
  await expect(card.getByRole("img",{name:"客户端自动接收.png",exact:true})).toBeVisible();
  await expect.poll(async()=>{
    const state = await(await request.get(b.address+"/api/state",{headers:ownerHeaders})).json();
    return state.transfers.find((t:any)=>t.id===sent.ids[0]).status;
  }).toBe("completed");
});

test("browser automatically downloads large images without keeping the full file in a Blob", async ({page,request}) => {
  const b = await boot();
  await page.goto(b.address);
  await page.getByLabel("六位配对码").fill(b.pairing_code);
  await page.getByRole("button",{name:"连接设备",exact:true}).click();
  await expect(page.getByRole("textbox",{name:"发送文字"})).toBeVisible();
  const token = await page.evaluate(()=>localStorage.getItem("linchuan-token"));
  const state = await(await request.get(b.address+"/api/state",{headers:{Authorization:"Bearer "+token}})).json();
  const headers = {Authorization:"Bearer "+b.token};
  const bytes = Buffer.alloc(4*1024**2+17,73);
  const draft = await(await request.post(b.address+"/api/drafts",{headers,data:{files:[{name:"大图片.heic",size:bytes.length,mime:"image/heic"}]}})).json();
  expect((await request.put(`${b.address}/api/drafts/${draft.id}/files/${draft.files[0].id}`,{headers,data:bytes})).ok()).toBeTruthy();
  const downloaded = page.waitForEvent("download");
  const sent = await(await request.post(`${b.address}/api/drafts/${draft.id}/send`,{headers,data:{text:"大图片自动下载",targets:[state.me.id]}})).json();
  const download = await downloaded;
  expect(download.suggestedFilename()).toBe("大图片.heic");
  expect(await readFile((await download.path())!)).toEqual(bytes);
  await expect.poll(async()=>{
    const state = await(await request.get(b.address+"/api/state",{headers})).json();
    return state.transfers.find((t:any)=>t.id===sent.ids[0]).status;
  }).toBe("completed");
  await expect(page.locator(".message").filter({hasText:"大图片自动下载"}).locator(".image-thumbnail")).toHaveCount(0);
});
