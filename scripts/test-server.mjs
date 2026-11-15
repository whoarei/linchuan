import { spawn } from 'node:child_process';
import { readFile, mkdir, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';
const root=path.resolve('.playwright');await rm(root,{recursive:true,force:true});await mkdir(root,{recursive:true});
const children=[];
for(const [index,name] of ['测试电脑','测试手机','临时测试电脑','HTTP测试电脑'].entries()){
 const dir=path.join(root,`device-${index}`);const child=spawn('target/debug/linchuan-core',[dir,name],{env:{...process.env,LINCHUAN_NO_DISCOVERY:'1'},stdio:['ignore','inherit','inherit']});children.push(child);
 let bootstrap;for(let attempt=0;attempt<100;attempt++){try{bootstrap=JSON.parse(await readFile(path.join(dir,'ready.json'),'utf8'));break;}catch{await new Promise(r=>setTimeout(r,100));}}
 if(!bootstrap)throw new Error('Core test server did not start');await writeFile(path.join(root,`bootstrap-${index}.json`),JSON.stringify(bootstrap));
}
const first=JSON.parse(await readFile(path.join(root,'bootstrap-0.json'),'utf8'));
const server=createServer((req,res)=>{if(req.url==='/health'){res.end('ready');}else{res.writeHead(302,{Location:first.address+(req.url||'/')});res.end();}});server.listen(1421,'127.0.0.1');
function stop(){for(const child of children)child.kill('SIGTERM');server.close();process.exit(0);}
process.on('SIGINT',stop);process.on('SIGTERM',stop);
