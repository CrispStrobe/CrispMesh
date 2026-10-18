import { describe, expect, it } from "vitest";
import { cropSamples, decodeForegroundPreview, paddedCrop, focusRegion, trackingRegion } from "./foregroundPreview";
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
