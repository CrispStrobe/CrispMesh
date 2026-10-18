// Exercise capture -> three reviewed masks -> full batch -> saved photos/masks/unmatched originals.
// The native bridge is a deterministic fixture; actual Vision inference is checked separately.
import { createServer } from 'vite';
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
const html=`<html><body><div id="root"></div><script type="module">
import {h,render} from 'preact';import {CaptureButton} from '/src/ui/capture.tsx';
import {setLanguage} from '/src/i18n/index.ts';import '/src/styles.css';setLanguage('en');
window.isTauri=true;window.calls=0;window.writes=[];
window.__TAURI_INTERNALS__={invoke:async(cmd,args)=>{
 if(cmd==='capture_foreground_available')return true;
 if(cmd==='camera_field_of_view')return 60;
 if(cmd!=='capture_foreground')throw Error(cmd);
 window.calls++;if(window.calls>=4&&window.calls<=7)throw 'foreground-not-found';
 const im=await createImageBitmap(new Blob([args],{type:'image/jpeg'}));const canvas=document.createElement('canvas');canvas.width=im.width;canvas.height=im.height;im.close();
 const x=Math.floor(canvas.width*.3),y=Math.floor(canvas.height*.2),w=Math.floor(canvas.width*.4),h=Math.floor(canvas.height*.6);
 const ctx=canvas.getContext('2d');ctx.fillStyle='black';ctx.fillRect(0,0,canvas.width,canvas.height);ctx.fillStyle='white';ctx.fillRect(x,y,w,h);
 const blob=await new Promise(done=>canvas.toBlob(done,'image/png'));const png=new Uint8Array(await blob.arrayBuffer());const out=new Uint8Array(24+png.length);const view=new DataView(out.buffer);
 [canvas.width,canvas.height,x,y,x+w,y+h].forEach((v,i)=>view.setUint32(i*4,v,true));out.set(png,24);return out.buffer;
}};
const engine={kind:'local',saveCapture:async(folder,files,progress)=>{window.writes.push({folder,names:files.map(f=>f.name)});progress?.(files.length,files.length);return folder;}};
render(h(CaptureButton,{engine,onSaved:(...args)=>window.saved=args}),document.getElementById('root'));
</script></body></html>`;
const server=await createServer({root:fileURLToPath(new URL('..',import.meta.url)),server:{host:'127.0.0.1',port:1434,strictPort:true,hmr:false},plugins:[{name:'capture-crop-fixture',configureServer(server){server.middlewares.use(async(req,res,next)=>{if(req.url!=='/__capture-crop')return next();res.setHeader('Content-Type','text/html');res.end(await server.transformIndexHtml(req.url,html));});}}]});await server.listen();
const browser=await chromium.launch({channel:'chrome',headless:true,args:['--use-fake-device-for-media-stream','--use-fake-ui-for-media-stream']});
try{
 const page=await browser.newPage({permissions:['camera'],viewport:{width:390,height:844}});await page.goto('http://127.0.0.1:1434/__capture-crop');
 await page.getByRole('button',{name:'Take photos',exact:true}).click();const dialog=page.locator('dialog.capture');
 await dialog.locator('.capture-auto input').uncheck();await page.waitForFunction(()=>document.querySelector('video')?.readyState>=2);await dialog.locator('video').evaluate(v=>v.pause());
 for(let i=0;i<16;i++){await dialog.getByRole('button',{name:'Take photo',exact:true}).click();await dialog.locator('.capture-thumb').nth(i).waitFor();}
 await dialog.getByText('Detect object and preview crop',{exact:true}).click();await page.waitForFunction(()=>window.calls===3);
 await dialog.locator('.crop-examples canvas').nth(2).waitFor();
 const use=dialog.getByRole('button',{name:'Use 16 photos',exact:true});await use.click();await page.waitForFunction(()=>window.saved);
 const result=await page.evaluate(()=>({saved:window.saved,calls:window.calls,writes:window.writes}));
 assert.equal(result.saved[1],15);assert.deepEqual(result.saved[3],[15]);assert.equal(result.calls,19,'reviewed samples are reused, with four failed search attempts for one omitted photo');
 assert.equal(result.writes.length,3);assert.equal(result.writes[0].names.length,15);assert.equal(result.writes[1].names.length,15);
 assert.equal(result.writes[1].names[0],'capture_0001.jpg.png');assert.equal(result.writes[2].names[0],'capture_0002.jpg');assert.ok(result.writes[2].folder.endsWith('-unmatched'));
 console.log('passed: reviewed masks reused; 15 accepted photos/masks renumbered; omitted original retained separately');
}finally{await browser.close();await server.close();}
