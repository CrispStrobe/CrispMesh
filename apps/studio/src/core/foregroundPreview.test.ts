import { describe, expect, it } from "vitest";
import { cropSamples, decodeForegroundPreview, paddedCrop, focusRegion, trackingRegion, recoverForegroundSpikes } from "./foregroundPreview";
function response(values = [3264,2448,700,300,1800,2000]) {
  const buffer = new ArrayBuffer(32);const view = new DataView(buffer);
  values.forEach((v,i)=>view.setUint32(4*i,v,true));
  new Uint8Array(buffer,24).set([137,80,78,71,13,10,26,10]);return buffer;
}
describe("native foreground previews",()=>{
  it("reads the source bounds without changing the camera frame",async()=>{
    const value=decodeForegroundPreview(response());
    expect([value.width,value.height,value.box]).toEqual([3264,2448,[700,300,1800,2000]]);
    expect(value.mask.type).toBe("image/png");expect(value.mask.size).toBe(8);
    expect(paddedCrop(value.box,value.width,value.height,.15)).toEqual([445,45,2055,2255]);
    expect(paddedCrop([0,10,100,100],100,100,.5)).toEqual([0,0,100,100]);
  });
  it("rejects invalid headers and out-of-frame bounds",()=>{
    for(const values of [[0,20,0,0,1,1],[20,20,10,1,10,2],[20,20,0,0,21,1]])expect(()=>decodeForegroundPreview(response(values))).toThrow();
    const invalid=response();new Uint8Array(invalid)[24]=0;expect(()=>decodeForegroundPreview(invalid)).toThrow();
    expect(()=>decodeForegroundPreview(new ArrayBuffer(24))).toThrow();
  });
  it("samples different distances and heights across a batch",()=>{
    expect(cropSamples([])).toEqual([]);expect(cropSamples([1])).toEqual([1]);expect(cropSamples([1,2])).toEqual([1,2]);
    expect(cropSamples(Array.from({length:119},(_,i)=>i+1))).toEqual([1,60,119]);
  });
});

it("keeps a tapped Auto area within the saved frame, including its edges",()=>{
  expect(focusRegion(.5,.5,.4)).toEqual([.3,.3,.7,.7]);
  expect(focusRegion(0,1,.4)).toEqual([0,.6,.4,1]);
  expect(focusRegion(-1,2,.4)).toEqual([0,.6,.4,1]);
});

it("keeps a tracking search in original normalized coordinates",()=>{
  const source=decodeForegroundPreview(response([1000,800,400,200,600,500]));
  expect(trackingRegion(source)).toEqual([.19,0,.81,.8875]);
});

function preview(box: [number,number,number,number]) {
  return decodeForegroundPreview(response([3024,4032,...box]));
}
it("recovers the real height-transition table selection using the following figure region",async()=>{
  const before=preview([1604,2313,1898,2822]);
  const wrong=preview([474,2717,2585,3970]);
  const after=preview([1652,2870,1958,3368]);
  const regions: unknown[]=[];
  const result=await recoverForegroundSpikes([before,wrong,after],async(index,region)=>{
    expect(index).toBe(1);regions.push(region);return after;
  });
  expect(regions).toEqual([trackingRegion(after)]);
  expect(result).toEqual([before,after,after]);
});
it("rejects a repeated table result and retains an unresolved slot for original-photo review",async()=>{
  const figure=preview([1600,2300,1900,2800]), table=preview([400,2600,2600,4000]);
  let attempts=0;
  expect(await recoverForegroundSpikes([figure,table,figure],async()=>{attempts++;return table;})).toEqual([figure,null,figure]);
  expect(attempts).toBe(2);
});
it("leaves sustained camera-distance changes and flat-object rotation alone",async()=>{
  const small=preview([1500,2400,1600,2900]), broad=preview([800,2400,2300,2900]);
  const large=preview([400,1000,2600,3900]);
  for (const frames of [[small,broad,small],[small,large,large],[null,large,small]]) {
    expect(await recoverForegroundSpikes(frames,async()=>{throw new Error('unexpected retry');})).toEqual(frames);
  }
});
