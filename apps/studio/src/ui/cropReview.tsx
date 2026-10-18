import { useEffect, useRef, useState } from "preact/hooks";
import { cropSamples, paddedCrop, trackingRegion, type CropSettings, type ForegroundPreview } from "../core/foregroundPreview";
import { foregroundPreview } from "../shell/shell";
import { t } from "../i18n";

export interface PreviewRow { file: File; result: ForegroundPreview | null; }
/** Three actual photos with their mask and proposed padded crop, before the full batch is processed. */
interface Props {
  files: File[];
  settings: CropSettings;
  onSettings(value: CropSettings): void;
  disabled?: boolean;
  onBusy?(busy: boolean): void;
  onResults?(rows: PreviewRow[], focus: number, threshold: number): void;
}
export function CropReview({ files, settings, onSettings, disabled = false, onBusy, onResults }: Props) {
  const callbacks = useRef({ onBusy, onResults });
  callbacks.current = { onBusy, onResults };
  const [rows, setRows] = useState<PreviewRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    if (disabled) { setBusy(false);callbacks.current.onBusy?.(false);return; }
    let alive = true;
    setProblem(false);setBusy(true);callbacks.current.onBusy?.(true);
    const timer = setTimeout(() => {
      void (async () => {
        try {
        const next: PreviewRow[] = [];
        let previous: ForegroundPreview | null = null;
        // Serial: model inference should not multiply memory/GPU work on a phone.
        for (const file of cropSamples(files)) {
          if (!alive) return;
          const result: ForegroundPreview | null = await foregroundPreview(file, settings.focus, settings.threshold, previous ? trackingRegion(previous) : undefined);
          if (result) previous = result;
          if (!alive) return;
          next.push({ file, result });
        }
        if (alive) {setRows(next);setBusy(false);callbacks.current.onResults?.(next, settings.focus, settings.threshold);callbacks.current.onBusy?.(false);}
        } catch {if(alive){setRows([]);setProblem(true);setBusy(false);callbacks.current.onResults?.([], settings.focus, settings.threshold);callbacks.current.onBusy?.(false);}}
      })();
    }, 250);
    return ()=>{alive=false;clearTimeout(timer);callbacks.current.onBusy?.(false);};
  }, [files, settings.focus, settings.threshold, revision, disabled]);
  return <fieldset class="crop-review" disabled={disabled}>
    <h2>{t("capture.crop.title")}</h2>
    <p class="help">{t("capture.crop.help")}</p>
    <label>{t("capture.crop.padding", { value: Math.round(settings.padding*100) })}
      <input type="range" min="0" max="0.5" step="0.01" value={settings.padding} onInput={e=>onSettings({...settings,padding:Number(e.currentTarget.value)})} />
    </label>
    <label>{t("capture.crop.focus")}
      <select value={settings.focus} onChange={e=>onSettings({...settings,focus:Number(e.currentTarget.value)})}>
        <option value="1">{t("capture.crop.whole")}</option><option value="0.7">{t("capture.crop.middle")}</option><option value="0.4">{t("capture.crop.tight")}</option>
      </select>
    </label>
    <label>{t("capture.crop.threshold", { value: Math.round(settings.threshold*100) })}
      <input type="range" min="0.2" max="0.8" step="0.05" value={settings.threshold} onInput={e=>onSettings({...settings,threshold:Number(e.currentTarget.value)})} />
    </label>
    <div class="crop-examples" aria-busy={busy}>
      {rows.map(row=><CropExample key={row.file.name} row={row} padding={settings.padding} />)}
    </div>
    <p role="status">{problem ? t("capture.crop.failed") : busy ? t("capture.crop.working") : t("capture.crop.legend")}</p>
    <button class="button" type="button" disabled={busy || !files.length} onClick={()=>setRevision(v=>v+1)}>{t("capture.crop.refresh")}</button>
  </fieldset>;
}

function CropExample({ row, padding }: { row: PreviewRow; padding: number }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(()=>{
    let alive=true;
    const urls: string[]=[];
    const load=(blob: Blob)=>new Promise<HTMLImageElement>((resolve,reject)=>{
      const image=new Image();const url=URL.createObjectURL(blob);urls.push(url);image.onload=()=>resolve(image);image.onerror=reject;image.src=url;
    });
    void (async()=>{
      try {
        const photo=await load(row.file);const mask=row.result ? await load(row.result.mask) : null;
        if(!alive || !canvas.current)return;
        const element=canvas.current;const scale=Math.min(1,480/photo.naturalWidth);
        element.width=Math.round(photo.naturalWidth*scale);element.height=Math.round(photo.naturalHeight*scale);
        const context=element.getContext("2d")!;context.drawImage(photo,0,0,element.width,element.height);
        if(mask && row.result) {
          const layer=document.createElement("canvas");layer.width=element.width;layer.height=element.height;
          const ctx=layer.getContext("2d")!;ctx.drawImage(mask,0,0,layer.width,layer.height);
          const pixels=ctx.getImageData(0,0,layer.width,layer.height);
          for(let i=0;i<pixels.data.length;i+=4){const foreground=pixels.data[i]!>127;pixels.data[i]=40;pixels.data[i+1]=220;pixels.data[i+2]=80;pixels.data[i+3]=foreground ? 100 : 0;}
          ctx.putImageData(pixels,0,0);context.drawImage(layer,0,0);
          const [x0,y0,x1,y1]=paddedCrop(row.result.box,row.result.width,row.result.height,padding);
          context.strokeStyle="#ffd34f";context.lineWidth=2;context.strokeRect(x0*scale,y0*scale,(x1-x0)*scale,(y1-y0)*scale);
        }
      } finally {for(const url of urls)URL.revokeObjectURL(url);}
    })().catch(()=>undefined);
    return ()=>{alive=false;for(const url of urls)URL.revokeObjectURL(url);};
  },[row,padding]);
  return <figure><canvas ref={canvas} aria-label={t("capture.crop.example",{ name:row.file.name })} /><figcaption>{row.file.name}{row.result===null && <span class="field-error">{t("capture.crop.notFound")}</span>}</figcaption></figure>;
}
