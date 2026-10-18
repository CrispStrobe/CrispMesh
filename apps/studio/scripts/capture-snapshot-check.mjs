// Compare the detector signals on saved photos. This does not replay video timing or prove physical angles.
// STUDIO_CAPTURE_DIR, STUDIO_MASKS_DIR (optional <name>.png, <stem>.png or <stem>-1.png), STUDIO_OUT (report directory).
import { createServer } from 'vite';
import { chromium } from 'playwright';
import { readdirSync, readFileSync, existsSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve, join, basename } from 'node:path';
if(!process.env.STUDIO_CAPTURE_DIR || !process.env.STUDIO_OUT)throw Error('STUDIO_CAPTURE_DIR and STUDIO_OUT are required');
const folder=resolve(process.env.STUDIO_CAPTURE_DIR);const files=readdirSync(folder).filter(n=>/\.jpe?g$/i.test(n)).sort();
const masks=files.map(name=>{if(!process.env.STUDIO_MASKS_DIR)return null;const stem=basename(name).replace(/\.jpe?g$/i,'');return [name+'.png',stem+'.png',stem+'-1.png'].map(file=>join(resolve(process.env.STUDIO_MASKS_DIR),file)).find(existsSync)??null;});
const rows=files.map((name,index)=>({name,photo:'/__capture/photo/'+index,mask:masks[index]&&existsSync(masks[index])?'/__capture/mask/'+index:null}));
const html=`<html><body><script type="module">
import {greyThumbnail,centralRegion,changedShare} from '/src/core/autoCapture.ts';
const rows=${JSON.stringify(rows)};const output=[];
const raw=(a,b)=>{let n=0;for(let i=0;i<a.length;i++)if(Math.abs(a[i]-b[i])>12)n++;return n/a.length;};
async function grey(blob,box){
 let image;
 if(box){const [l,t,r,b]=box;const factor=96/Math.max(r-l,b-t);image=await createImageBitmap(blob,l,t,r-l,b-t,{resizeWidth:Math.max(1,Math.round((r-l)*factor)),resizeHeight:Math.max(1,Math.round((b-t)*factor)),resizeQuality:'low'});}
 else {const original=await createImageBitmap(blob);const factor=96/Math.max(original.width,original.height);image=await createImageBitmap(original,{resizeWidth:Math.max(1,Math.round(original.width*factor)),resizeHeight:Math.max(1,Math.round(original.height*factor)),resizeQuality:'low'});original.close();}
 const canvas=document.createElement('canvas');canvas.width=image.width;canvas.height=image.height;const context=canvas.getContext('2d');context.drawImage(image,0,0);image.close();
 return greyThumbnail(context.getImageData(0,0,canvas.width,canvas.height).data,canvas.width,canvas.height);
}
async function region(url){
 if(!url)return null;
 const buffer=await(await fetch(url)).arrayBuffer();const view=new DataView(buffer);const width=view.getUint32(16),height=view.getUint32(20);
 const mask=await grey(new Blob([buffer],{type:'image/png'}));let left=mask.width,top=mask.height,right=0,bottom=0,area=0;
 for(let y=0;y<mask.height;y++)for(let x=0;x<mask.width;x++)if(mask.data[y*mask.width+x]>127){left=Math.min(left,x);top=Math.min(top,y);right=Math.max(right,x+1);bottom=Math.max(bottom,y+1);area++;}
 if(!area)return null;const margin=Math.ceil(.35*Math.max((right-left)/mask.width*width,(bottom-top)/mask.height*height));
 return {box:[Math.max(0,Math.floor(left/mask.width*width)-margin),Math.max(0,Math.floor(top/mask.height*height)-margin),Math.min(width,Math.ceil(right/mask.width*width)+margin),Math.min(height,Math.ceil(bottom/mask.height*height)+margin)],foregroundShare:area/(mask.width*mask.height)};
}
let previous=null;
for(const [index,row] of rows.entries()){
 const blob=await(await fetch(row.photo,{cache:'no-store'})).blob();const full=await grey(blob);const centre=centralRegion(full.data,full.width,full.height);const roi=await region(row.mask);
 if(previous){const entry={from:previous.name,to:row.name,oldRawFull:raw(previous.full.data,full.data),exposureNormalisedCentre:changedShare(previous.centre,centre),previousForegroundShare:previous.roi?.foregroundShare??null};
  if(previous.roi){const a=await grey(previous.blob,previous.roi.box);const b=await grey(blob,previous.roi.box);entry.exposureNormalisedObject=changedShare(a.data,b.data);}
  output.push(entry);
 }
 previous={name:row.name,blob,full,centre,roi};
}
window.captureReport={note:'Saved-image signal comparison; image bitmap resampling may differ from live camera frames. No video timing or calibrated turntable angles. Object ROI is the previous photo foreground box plus35% margin.',pairs:output};
</script></body></html>`;
const server=await createServer({server:{port:1432,strictPort:true,hmr:false},plugins:[{name:'snapshot-check',configureServer(server){server.middlewares.use(async(req,res,next)=>{
 const match=req.url?.match(/^\/__capture\/(photo|mask)\/(\d+)$/);
 if(match){const i=Number(match[2]);const path=match[1]==='photo'&&files[i]?join(folder,files[i]):masks[i];if(!path||!existsSync(path)){res.statusCode=404;return res.end();}res.setHeader('Cache-Control','no-store');res.setHeader('Content-Type',match[1]==='photo'?'image/jpeg':'image/png');return res.end(readFileSync(path));}
 if(req.url==='/__snapshot-check'){res.setHeader('Content-Type','text/html');return res.end(await server.transformIndexHtml(req.url,html));}next();
});}}]});await server.listen();const browser=await chromium.launch({channel:'chrome',headless:true});
try{const page=await browser.newPage();page.on('pageerror',error=>console.error(error.message));await page.goto('http://127.0.0.1:1432/__snapshot-check');await page.waitForFunction(()=>window.captureReport,null,{timeout:240000});const report=await page.evaluate(()=>window.captureReport);mkdirSync(process.env.STUDIO_OUT,{recursive:true});writeFileSync(join(process.env.STUDIO_OUT,'snapshot-detector-signals.json'),JSON.stringify(report,null,2));console.log('Compared',report.pairs.length,'consecutive saved-photo pairs.');}finally{await browser.close();await server.close();}
