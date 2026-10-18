// The crop controls and overlays, using a deterministic native bridge fixture (no model/GPU).
import { createServer } from 'vite';
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
const html=`<html><head><meta name="viewport" content="width=device-width, initial-scale=1"></head><body><main id="root" class="page narrow"></main><script type="module">
import {h,render} from 'preact';
import {useState} from 'preact/hooks';
import {CropReview} from '/src/ui/cropReview.tsx';
import {setLanguage} from '/src/i18n/index.ts';
import '/src/styles.css';
setLanguage(new URLSearchParams(location.search).get('lang')||'en');
const canvas=document.createElement('canvas');canvas.width=320;canvas.height=240;
const ctx=canvas.getContext('2d');ctx.fillStyle='white';ctx.fillRect(0,0,320,240);ctx.fillStyle='#b14336';ctx.fillRect(100,40,120,160);
const blob=await new Promise(done=>canvas.toBlob(done,'image/jpeg'));
const files=[1,60,119].map(i=>new File([blob],'photo-'+i+'.jpg',{type:'image/jpeg'}));
ctx.fillStyle='black';ctx.fillRect(0,0,320,240);ctx.fillStyle='white';ctx.fillRect(100,40,120,160);
const mask=await new Promise(done=>canvas.toBlob(done,'image/png'));
const png=new Uint8Array(await mask.arrayBuffer());
window.calls=[];window.__TAURI_INTERNALS__={invoke:async(cmd,args,options)=>{
 if(cmd!=='capture_foreground')throw Error(cmd);window.calls.push(options.headers);
 const out=new Uint8Array(24+png.length);const view=new DataView(out.buffer);
 [320,240,100,40,220,200].forEach((v,i)=>view.setUint32(i*4,v,true));out.set(png,24);return out.buffer;
}};
function App(){const [settings,setSettings]=useState({padding:.15,focus:1,threshold:.5});return h(CropReview,{files,settings,onSettings:setSettings});}
render(h(App),document.getElementById('root'));
</script></body></html>`;
const server=await createServer({server:{port:1432,strictPort:true},plugins:[{name:'crop-review-fixture',configureServer(server){server.middlewares.use(async(req,res,next)=>{if(!req.url?.startsWith('/__crop-review'))return next();try{res.setHeader('Content-Type','text/html');res.end(await server.transformIndexHtml(req.url,html));}catch(error){next(error);}});}}]});
await server.listen();
const browser=await chromium.launch({channel:'chrome',headless:true});
try{
 for(const lang of ['en','de']){
  const page=await browser.newPage({viewport:{width:390,height:844}});
  await page.goto('http://127.0.0.1:1432/__crop-review?lang='+lang);
  await page.locator('.crop-examples canvas').nth(2).waitFor();
  await page.waitForFunction(()=>window.calls?.length===3);
  const first=await page.locator('.crop-examples canvas').first().evaluate(canvas=>Array.from(canvas.getContext('2d').getImageData(100,60,1,1).data));
  assert.ok(first[1]>100,'foreground is shown with a green overlay');
  await page.locator('input[type=range]').first().fill('0.3');
  await page.waitForTimeout(350);assert.equal(await page.evaluate(()=>window.calls.length),3,'padding changes the crop without rerunning masks');
  await page.locator('select').selectOption('0.7');
  await page.waitForFunction(()=>window.calls?.length===6);
  assert.equal(await page.evaluate(()=>window.calls.at(-1)['x-focus']),'0.7');
  await page.locator('input[type=range]').nth(1).fill('0.65');
  await page.waitForFunction(()=>window.calls?.length===9);
  assert.equal(await page.evaluate(()=>window.calls.at(-1)['x-threshold']),'0.65');
  if(process.env.STUDIO_OUT)await page.screenshot({path:process.env.STUDIO_OUT+'/crop-controls-'+lang+'.png',fullPage:true});
  console.log(lang,'passed: three sample photos, mask overlay, padding/focus/threshold');
  await page.close();
 }
}finally{await browser.close();await server.close();}
