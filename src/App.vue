<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { ArrowUpRight, ArrowDownLeft, Check, CheckCheck, Clipboard, Download, File, FileImage, Globe, Laptop, Link, LoaderCircle, Monitor, Plus, QrCode, Send, Settings, ShieldCheck, Smartphone, Upload, Wifi, X, Inbox, ArrowLeft, Copy, Radio } from 'lucide-vue-next';
import QRCode from 'qrcode';
import { useModalFocus } from './useModalFocus';
import { api, bootstrap, native, token, setToken, forget, pairingCode, address, upload, ApiError } from './api';
import type { AppState, FileMeta, Transfer } from './types';
const state=ref<AppState>();const ready=ref(false);const bootError=ref('');const connected=ref(true);const loading=ref(true);
const name=ref('');const code=ref('');const hostName=ref('邻传设备');const pairError=ref('');const pairing=ref(false);
const tab=ref<'send'|'inbox'|'history'>('send');const selected=ref<string[]>([]);const text=ref('');const files=ref<File[]>([]);const dragging=ref(false);
const busy=ref(false);const uploaded=ref(0);const uploadTotal=ref(0);const controller=ref<AbortController>();const fileInput=ref<HTMLInputElement>();
const connection=ref(false);const manual=ref(false);const manualAddress=ref('');const manualBusy=ref(false);const qr=ref('');const shareAddress=ref('');const toast=ref('');const toastError=ref(false);let toastTimer:ReturnType<typeof setTimeout>;
const editName=ref(false);const newName=ref('');const preview=ref<{url:string;name:string}>();
const transfers=computed(()=>state.value?.transfers||[]);
const incoming=computed(()=>transfers.value.filter(t=>t.recipient_id===state.value?.me.id));
const pending=computed(()=>incoming.value.filter(t=>t.status==='pending'));
const active=computed(()=>transfers.value.filter(t=>!terminal(t.status)));
const visible=computed(()=>tab.value==='inbox'?incoming.value:transfers.value);
const textBytes=computed(()=>new TextEncoder().encode(text.value).length);
const textTooLong = computed(() => textBytes.value > 65_536);
const canSend = computed(() =>
 selected.value.length > 0 &&
 (text.value.trim().length > 0 || files.value.length > 0) &&
 !busy.value && !textTooLong.value,
);
const modalKind = computed(() => connection.value ? 'connection' : manual.value ? 'manual' : editName.value ? 'name' : preview.value ? 'preview' : '');
const modalContainer = ref<HTMLElement>();
function closeModal() {
 connection.value = false;
 manual.value = false;
 editName.value = false;
 closePreview();
}
useModalFocus(modalKind, modalContainer, closeModal);
function disconnect() {
 forget();
 ready.value = false;
 selected.value = [];
 socket?.close();
}
const totalSize=computed(()=>files.value.reduce((sum,file)=>sum+file.size,0));
let poll:ReturnType<typeof setInterval>;let socket:WebSocket|undefined;let disposed=false;let refreshing=false;
function notify(message:string,error=false){toast.value=message;toastError.value=error;clearTimeout(toastTimer);toastTimer=setTimeout(()=>toast.value='',4500);}
function terminal(status:string){return ['completed','rejected','cancelled','failed'].includes(status);}
function size(bytes:number){if(bytes===0)return '0 B';const u=['B','KB','MB','GB'];const i=Math.min(Math.floor(Math.log(bytes)/Math.log(1024)),3);return `${(bytes/1024**i).toFixed(i>0?1:0)} ${u[i]}`;}
function statusLabel(status:string){return ({pending:'等待接收',accepted:'已接受',transferring:'正在传输',completed:'已完成',rejected:'已拒绝',cancelled:'已取消',failed:'传输失败'} as Record<string,string>)[status]||status;}
function time(value:number){return new Date(value*1000).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'});}
function toggle(id:string){if(busy.value)return;if(selected.value.includes(id))selected.value=selected.value.filter(v=>v!==id);else if(selected.value.length<8)selected.value.push(id);else notify('一次最多选择 8 台设备',true);}
async function refresh(){
 if(refreshing||disposed)return;refreshing=true;
 try{const value=await api<AppState>('/api/state');if(disposed)return;state.value=value;ready.value=true;connected.value=true;selected.value=selected.value.filter(id=>value.devices.some(d=>d.id===id));if(!shareAddress.value)shareAddress.value=value.info.addresses[0]||address();}
 catch(error){if(disposed)return;if(error instanceof ApiError&&error.status===401&&!native){forget();ready.value=false;socket?.close();}else connected.value=false;}
 finally{refreshing=false;loading.value=false;}
}
function openEvents(){
 if(socket&&(socket.readyState===WebSocket.OPEN||socket.readyState===WebSocket.CONNECTING))return;
 socket=new WebSocket(address().replace(/^http/,'ws')+'/api/events');socket.onopen=()=>socket?.send(JSON.stringify({token:token()}));socket.onmessage=()=>void refresh();socket.onerror=()=>{};
}
async function pair(){pairing.value=true;pairError.value='';try{const result=await api<{token:string}>('/api/pair',{code:code.value,name:name.value});setToken(result.token);await refresh();if(ready.value)openEvents();}catch(error){pairError.value=(error as Error).message;}finally{pairing.value=false;}}
function addFiles(items:File[]){
 const valid:File[]=[];for(const file of items){if(file.size>2*1024**3){notify(`${file.name} 超过 2 GiB`,true);continue;}if(files.value.length+valid.length>=64){notify('一次最多添加 64 个文件',true);break;}if(file.name.includes('/')||file.name.includes('\\')||file.name.includes(':')){notify('文件名不能包含路径符号',true);continue;}valid.push(file);}files.value.push(...valid);
}
function fileChange(event:Event){const input=event.target as HTMLInputElement;addFiles(Array.from(input.files||[]));input.value='';}
function drop(event:DragEvent){event.preventDefault();dragging.value=false;if(!busy.value)addFiles(Array.from(event.dataTransfer?.files||[]));}
function paste(event:ClipboardEvent){const items=Array.from(event.clipboardData?.items||[]);const images=items.filter(item=>item.kind==='file').map(item=>item.getAsFile()).filter((f):f is File=>f!==null);if(images.length&&!busy.value){addFiles(images);event.preventDefault();}}
async function send(){
 if(!canSend.value)return;busy.value=true;uploaded.value=0;uploadTotal.value=totalSize.value;const targets=[...selected.value];const message=text.value;const abort=new AbortController();controller.value=abort;
 try{const draft=await api<{id:string;files:FileMeta[]}>('/api/drafts',{files:files.value.map(f=>({name:f.name,size:f.size,mime:f.type}))});let completed=0;
 for(let i=0;i<files.value.length;i++){const file=files.value[i];await upload(`/api/drafts/${draft.id}/files/${draft.files[i].id}`,file,bytes=>uploaded.value=completed+bytes,abort.signal);completed+=file.size;}
 if(abort.signal.aborted)throw new Error('已取消上传');
 await api(`/api/drafts/${draft.id}/send`,{targets,text:message});files.value=[];text.value='';notify('已发送请求，等待对方接收');await refresh();tab.value='history';
 }catch(error){notify((error as Error).message,true);}finally{busy.value=false;controller.value=undefined;}
}
async function decide(transfer:Transfer,accept:boolean){try{await api(`/api/transfers/${transfer.id}/decision`,{accept});await refresh();}catch(error){notify((error as Error).message,true);}}
async function cancel(transfer:Transfer){try{await api(`/api/transfers/${transfer.id}/cancel`,{});await refresh();}catch(error){notify((error as Error).message,true);}}
async function download(transfer:Transfer,file:FileMeta){try{const result=await api<{url:string}>(`/api/transfers/${transfer.id}/files/${file.id}/ticket`,{});const url=address()+result.url;if(native){const {openUrl}=await import('@tauri-apps/plugin-opener');await openUrl(url);}else{const a=document.createElement('a');a.href=url;a.download=file.name;document.body.append(a);a.click();a.remove();}}catch(error){notify((error as Error).message,true);}}
async function viewImage(transfer:Transfer,file:FileMeta){
 if(file.size>20*1024**2){notify('大于 20 MB 的图片请下载后查看');return;}
 try{const result=await api<{url:string}>(`/api/transfers/${transfer.id}/files/${file.id}/ticket`,{});const response=await fetch(address()+result.url);if(!response.ok)throw new Error('图片预览失败');const blob=await response.blob();if(preview.value)URL.revokeObjectURL(preview.value.url);preview.value={url:URL.createObjectURL(new Blob([blob],{type:file.mime})),name:file.name};}catch(error){notify((error as Error).message,true);}
}
function closePreview(){if(preview.value)URL.revokeObjectURL(preview.value.url);preview.value=undefined;}
async function copy(value:string){try{if(navigator.clipboard&&window.isSecureContext)await navigator.clipboard.writeText(value);else{const input=document.createElement('textarea');input.value=value;input.style.position='fixed';input.style.opacity='0';document.body.append(input);input.select();const ok=document.execCommand('copy');input.remove();if(!ok)throw new Error('copy');}notify('已复制');}catch{notify('当前浏览器无法自动复制，请长按或选中文字复制',true);}}
async function showConnection(){connection.value=true;qr.value=await QRCode.toDataURL(shareAddress.value,{width:240,margin:1,color:{dark:'#25334d',light:'#ffffff'}});}
async function changeAddress(){qr.value=await QRCode.toDataURL(shareAddress.value,{width:240,margin:1});}
async function connectPeer(){manualBusy.value=true;try{const result=await api<{id:string}>('/api/peers',{address:manualAddress.value.trim()});await refresh();if(!selected.value.includes(result.id))selected.value.push(result.id);manual.value=false;manualAddress.value='';notify('设备已连接');}catch(error){notify((error as Error).message,true);}finally{manualBusy.value=false;}}
async function saveName(){try{await api('/api/settings',{name:newName.value});editName.value=false;await refresh();notify('名称已更新，设备发现名称将在下次启动后更新');}catch(error){notify((error as Error).message,true);}}
onMounted(async()=>{try{await bootstrap();const info=await api<{name:string}>('/api/info');hostName.value=info.name;if(token())await refresh();else loading.value=false;if(ready.value)openEvents();poll=setInterval(()=>{if(token()){void refresh();if(ready.value)openEvents();}},3000);}catch(error){bootError.value=(error as Error).message;loading.value=false;}});
onUnmounted(()=>{disposed=true;clearInterval(poll);clearTimeout(toastTimer);socket?.close();controller.value?.abort();closePreview();});
</script>

<template>
 <div v-if="loading || bootError" class="launch"><div class="brand-mark">邻</div><h1>邻传</h1><p v-if="bootError" class="error">启动失败：{{ bootError }}</p><template v-else><LoaderCircle class="spin"/><p>正在准备你的传送空间…</p></template></div>
 <main v-else-if="!ready" class="pair-page"><div class="pair-card"><div class="brand-mark">邻</div><div class="eyebrow">LINCHUAN / 邻传</div><h1>连接设备</h1><p>连接 <strong>{{ hostName }}</strong>，传送文字、图片和文件。</p><form @submit.prevent="pair"><label>你的设备名称<input v-model="name" required maxlength="64" placeholder="例如：小林的手机" autocomplete="nickname"/></label><label>六位配对码<input v-model="code" required inputmode="numeric" pattern="[0-9]{6}" maxlength="6" placeholder="查看对方应用中的配对码" autocomplete="off"/></label><p v-if="pairError" class="error" role="alert">{{ pairError }}</p><button class="primary wide" :disabled="pairing"><LoaderCircle v-if="pairing" class="spin" :size="18"/><Link v-else :size="18"/>{{ pairing?'正在连接':'连接设备' }}</button></form><div class="privacy-note"><ShieldCheck :size="16"/>无需账号 · 无云端中转</div><small>仅在可信局域网使用 · HTTP 传输</small></div></main>
 <div v-else class="app-shell" :inert="!!modalKind">
  <aside class="sidebar">
   <a class="brand" href="#" @click.prevent="tab='send'"><span class="brand-mark">邻</span><span><b>邻传</b><small>LINCHUAN</small></span><span class="version">0.1</span></a>
   <div class="workspace-label">工作区</div>
   <nav class="navigation"><button :class="{current:tab==='send'}" @click="tab='send'"><Send :size="18"/>发送内容<ArrowUpRight v-if="tab==='send'" :size="16"/></button><button :class="{current:tab==='inbox'}" @click="tab='inbox'"><Inbox :size="18"/>收件箱<span v-if="pending.length" class="badge">{{ pending.length }}</span></button><button :class="{current:tab==='history'}" @click="tab='history'"><Radio :size="18"/>传输记录<span v-if="active.length" class="count">{{ active.length }}</span></button></nav>
   <div class="device-heading"><span>附近的设备</span><span class="online-dot" title="设备在线"/><span class="device-count">{{ state!.devices.length }}</span></div>
   <div class="device-list"><button v-for="device in state!.devices" :key="device.id" class="device" :class="{selected:selected.includes(device.id)}" @click="toggle(device.id);tab='send'"><span class="device-icon"><Globe v-if="device.kind==='browser'" :size="21"/><Smartphone v-else-if="device.platform==='Android'" :size="21"/><Monitor v-else :size="21"/></span><span class="device-text"><b>{{ device.name }}</b><small>{{ device.platform }}<template v-if="device.kind==='host'"> · 当前连接</template></small></span><span class="selection"><Check v-if="selected.includes(device.id)" :size="13"/></span></button><div v-if="!state!.devices.length" class="device-empty"><Wifi :size="26"/><p>等待附近设备加入</p><small>打开另一台设备上的邻传，或让浏览器访问本机地址。</small></div></div>
   <button class="manual-connect" @click="manual=true"><Plus :size="16"/>通过地址连接</button>
   <div class="sidebar-bottom"><div class="local-device"><span class="local-avatar"><Laptop :size="20"/></span><span><b>{{ state!.me.name }}</b><small><i/>{{ state!.me.owner?'本机设备':'浏览器会话' }}</small></span><button v-if="state!.me.owner" class="icon-button" title="修改设备名称" @click="newName=state!.me.name;editName=true"><Settings :size="16"/></button></div><button v-if="state!.me.owner" class="connection-button" @click="showConnection"><QrCode :size="17"/>让其他设备连接<ArrowUpRight :size="15"/></button><button v-else class="connection-button" @click="disconnect"><ArrowLeft :size="16"/>断开当前会话</button></div>
  </aside>
  <section class="main-panel">
   <header class="topbar"><div class="breadcrumb">我的空间<span>/</span><b>{{ tab==='send'?'发送内容':tab==='inbox'?'收件箱':'传输记录' }}</b></div><div class="network-status" :class="{offline:!connected}"><span class="online-dot"/>{{ connected?'局域网已连接':'连接中断，正在重试' }}</div></header>
   <div class="content">
    <template v-if="tab==='send'">
     <div class="page-heading"><h1>发送内容</h1><p>选择附近设备，发送文字、图片或文件。</p></div>
     <div v-if="pending.length" class="incoming-banner" @click="tab='inbox'" role="button" tabindex="0" @keydown.enter="tab='inbox'"><Inbox :size="20"/><span>你有 <b>{{ pending.length }}</b> 个待接收请求</span><ArrowUpRight :size="18"/></div>
     <div class="compose-card">
      <div class="compose-top"><span class="step-label">添加内容</span></div>
      <textarea v-model="text" aria-label="发送文字" :aria-invalid="textTooLong" :aria-describedby="textTooLong ? 'text-limit-error' : undefined" placeholder="写点什么，或粘贴一段文字、网址…" :disabled="busy" @paste="paste"/>
      <p v-if="textTooLong" id="text-limit-error" class="error text-limit-error" role="alert">文字超过 65,536 字节，请缩短后再发送。文件尚未上传。</p>
      <div class="drop-zone" :class="{dragging}" @dragover.prevent="dragging=true" @dragleave.prevent="dragging=false" @drop="drop" @paste="paste"><span class="upload-icon"><Upload :size="23"/></span><div><b>把文件拖到这里，或 <button class="text-button" :disabled="busy" @click="fileInput?.click()">选择文件</button></b><p>支持多文件、原图，也可以直接粘贴截图</p></div><span class="file-limit">单文件 ≤ 2 GiB</span><input ref="fileInput" type="file" multiple hidden :disabled="busy" @change="fileChange"/></div>
      <div v-if="files.length" class="selected-files"><div v-for="(file,index) in files" :key="index" class="file-row"><FileImage v-if="file.type.startsWith('image/')" :size="19"/><File v-else :size="19"/><span>{{ file.name }}<small>{{ size(file.size) }}</small></span><button :disabled="busy" class="icon-button" :aria-label="`移除 ${file.name}`" @click="files.splice(index,1)"><X :size="16"/></button></div></div>
      <div class="compose-bottom"><span><Clipboard :size="14"/>内容只在你的局域网内传输</span><span v-if="files.length">{{ files.length }} 个文件 · {{ size(totalSize) }}</span><span v-else>{{ textBytes.toLocaleString() }} / 65,536 字节</span></div>
     </div>
     <div class="recipient-card"><div class="recipient-info"><span class="step-label">发送给谁</span><p v-if="selected.length">已选择 {{ selected.length }} 台设备 · 每位接收者独立确认</p><p v-else>从{{ state!.devices.length?'设备列表':'附近设备列表' }}中选择一台或多台设备</p></div><div class="recipient-chips"><button v-for="id in selected" :key="id" @click="toggle(id)">{{ state!.devices.find(d=>d.id===id)?.name }}<X :size="12"/></button><span v-if="!selected.length" class="muted small">尚未选择</span></div></div>
     <div v-if="busy" class="upload-progress"><div><span>正在准备文件… {{ size(uploaded) }} / {{ size(uploadTotal) }}</span><button class="text-button" @click="controller?.abort()">取消上传</button></div><progress :max="uploadTotal||1" :value="uploaded"/></div>
     <div class="send-actions"><button class="primary send-button" :disabled="!canSend" @click="send"><LoaderCircle v-if="busy" class="spin" :size="18"/><Send v-else :size="18"/>{{ busy?'正在准备':'发送内容' }}<span v-if="selected.length&&!busy">{{ selected.length }}</span></button></div>

    </template>
    <template v-else>
     <div class="page-heading"><h1>{{ tab==='inbox'?'收件箱':'传输记录' }}</h1><p>{{ tab==='inbox'?'先确认，再接收。收到的内容都在这里。':'查看本次运行的收发进度与结果。' }}</p></div>
     <div v-if="!visible.length" class="empty-state"><span><Inbox v-if="tab==='inbox'" :size="34"/><Radio v-else :size="34"/></span><h3>{{ tab==='inbox'?'收件箱还是空的':'还没有传输记录' }}</h3><p>让附近的设备给你发送内容，或开始你的第一次分享。</p><button class="secondary" @click="tab='send'">去发送内容<ArrowUpRight :size="16"/></button></div>
     <article v-for="transfer in visible" :key="transfer.id" class="transfer-card"><div class="transfer-header"><span class="transfer-direction"><ArrowDownLeft v-if="transfer.recipient_id===state!.me.id" :size="19"/><ArrowUpRight v-else :size="19"/></span><div><b>{{ transfer.recipient_id===state!.me.id?transfer.sender_name:transfer.recipient_name }}</b><small>{{ transfer.recipient_id===state!.me.id?'发给你':'你发送给对方' }} · {{ time(transfer.created) }}</small></div><span class="status" :class="transfer.status"><CheckCheck v-if="transfer.status==='completed'" :size="14"/><LoaderCircle v-if="transfer.status==='transferring'" class="spin" :size="14"/>{{ statusLabel(transfer.status) }}</span></div>
      <div v-if="transfer.text" class="received-text"><p>{{ transfer.text }}</p><button v-if="transfer.status==='completed'" class="icon-button" title="复制文字" @click="copy(transfer.text)"><Copy :size="16"/></button></div>
      <div v-for="file in transfer.files" :key="file.id" class="received-file"><span class="file-type"><FileImage v-if="file.mime.startsWith('image/')" :size="21"/><File v-else :size="21"/></span><span><b>{{ file.name }}</b><small>{{ size(file.size) }}</small></span><template v-if="transfer.status==='completed'"><button v-if="/^image\/(png|jpeg|gif|webp|bmp)$/.test(file.mime)" class="text-button" @click="viewImage(transfer,file)">预览</button><button class="icon-button" :aria-label="`下载 ${file.name}`" @click="download(transfer,file)"><Download :size="18"/></button></template></div>
      <p v-if="transfer.error" class="error">{{ transfer.error }}</p><div v-if="transfer.status==='transferring'" class="transfer-progress"><progress :value="transfer.bytes" :max="transfer.total||1"/><small>{{ size(transfer.bytes) }} / {{ size(transfer.total) }}</small></div>
      <div v-if="transfer.status==='pending'&&transfer.recipient_id===state!.me.id" class="transfer-actions"><span>确认接收来自这台设备的内容？</span><button class="secondary" @click="decide(transfer,false)">拒绝</button><button class="primary" @click="decide(transfer,true)"><Check :size="16"/>接收</button></div><div v-else-if="!terminal(transfer.status)" class="transfer-actions"><span>{{ transfer.status==='pending'?'等待对方确认接收…':'保持应用或网页打开，传输正在进行。' }}</span><button class="secondary" @click="cancel(transfer)">取消传输</button></div>
     </article>
    </template>
    <footer><span>邻传 · Linchuan 0.1.0</span><span>局域网传输</span></footer>
   </div>
  </section>
  <nav class="mobile-nav"><button :class="{current:tab==='send'}" @click="tab='send'"><Send :size="20"/>发送</button><button :class="{current:tab==='inbox'}" @click="tab='inbox'"><Inbox :size="20"/>收件箱 {{ pending.length||'' }}</button><button :class="{current:tab==='history'}" @click="tab='history'"><Radio :size="20"/>记录</button><button v-if="state!.me.owner" @click="showConnection"><QrCode :size="20"/>连接</button><button v-else aria-label="断开当前会话" @click="disconnect"><ArrowLeft :size="20"/>断开</button></nav>
 </div>
 <div v-if="toast" class="toast" :class="{error:toastError}" role="status"><X v-if="toastError" :size="18"/><Check v-else :size="18"/>{{ toast }}</div>
 <div v-if="modalKind" ref="modalContainer" class="modal-backdrop" tabindex="-1" @click.self="closeModal">
  <section v-if="connection" class="modal connection-modal" role="dialog" aria-modal="true" aria-labelledby="modal-title"><button class="modal-close icon-button" aria-label="关闭" @click="connection=false"><X :size="20"/></button><div class="modal-icon"><QrCode :size="25"/></div><h2 id="modal-title">连接其他设备</h2><p>另一台设备打开浏览器，扫码或输入下方地址。</p><div class="connection-device-name"><span>{{ state?.me.name }}</span><button class="text-button" @click="newName=state!.me.name;connection=false;editName=true">修改名称</button></div><img :src="qr" alt="本机浏览器访问二维码" class="qr-image"/><select v-if="(state?.info.addresses.length||0)>1" v-model="shareAddress" aria-label="选择网络地址" @change="changeAddress"><option v-for="item in state?.info.addresses" :key="item">{{ item }}</option></select><button class="address-copy" @click="copy(shareAddress)"><span>{{ shareAddress }}</span><Copy :size="16"/></button><div class="pairing-code"><span>本次配对码</span><b>{{ pairingCode }}</b><small>仅分享给你希望连接的人</small></div><div class="notice">保持应用运行。仅在可信局域网使用，首版采用 HTTP 传输。</div></section>
  <section v-else-if="manual" class="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title"><button class="modal-close icon-button" aria-label="关闭" @click="manual=false"><X :size="20"/></button><div class="modal-icon"><Link :size="24"/></div><h2 id="modal-title">通过地址连接</h2><p>在对方应用的连接面板中找到完整访问地址。</p><form @submit.prevent="connectPeer"><label>对方设备地址<input v-model="manualAddress" required placeholder="http://192.168.1.20:52783" type="url"/></label><button class="primary wide" :disabled="manualBusy"><LoaderCircle v-if="manualBusy" class="spin" :size="18"/>{{ manualBusy?'正在连接':'连接设备' }}</button></form></section>
  <section v-else-if="editName" class="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title"><button class="modal-close icon-button" aria-label="关闭" @click="editName=false"><X :size="20"/></button><h2 id="modal-title">给设备起个名字</h2><p>设备名称会显示在附近设备列表中。</p><form @submit.prevent="saveName"><label>设备名称<input v-model="newName" required maxlength="64"/></label><button class="primary wide">保存名称</button></form></section>
  <section v-else-if="preview" class="modal image-modal" role="dialog" aria-modal="true" aria-labelledby="modal-title"><button class="modal-close icon-button" aria-label="关闭" @click="closePreview"><X :size="20"/></button><h3 id="modal-title">{{ preview.name }}</h3><img :src="preview.url" :alt="preview.name"/></section>
 </div>
</template>
