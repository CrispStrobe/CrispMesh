// Capture review in English/German, using Chrome's fake camera. No reconstruction job.
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
const browser=await chromium.launch({channel:'chrome',headless:true,args:['--use-fake-device-for-media-stream','--use-fake-ui-for-media-stream','--enable-unsafe-webgpu']});
try {
 for(const locale of ['en-US','de-DE']) {
  const de=locale==='de-DE';
  const context=await browser.newContext({locale,permissions:['camera'],viewport:{width:390,height:844}});
  const page=await context.newPage();
  await page.goto((process.env.STUDIO_URL??'http://127.0.0.1:1431/')+'#/');
  const card=page.locator('section.card',{has:page.getByRole('heading',{name:de?/Dieser Browser|Im Browser rekonstruieren/:/This browser|Reconstruct in this browser/})});
  await card.getByRole('link').first().click();
  await page.getByRole('heading',{name:de?'Rekonstruktionen':'Runs',exact:true}).waitFor();
  await page.getByRole('link',{name:de?/Neue Rekonstruktion|Erste Rekonstruktion/:/New run|Start the first run/}).first().click();
  await page.locator('#s-sizes').waitFor({timeout:60000});
  await page.locator('.segmented button',{hasText:de?'Drehteller-Fotos':'Turntable photos'}).click();
  await page.getByRole('button',{name:de?'Fotos aufnehmen':'Take photos',exact:true}).click();
  const dialog=page.locator('dialog.capture');
  await dialog.locator('.capture-auto input').uncheck();
  await dialog.locator('.capture-zoom input').fill('2');
  await page.waitForFunction(()=>document.querySelector('dialog.capture video')?.readyState>=2);
  await dialog.locator('video').evaluate(video=>video.pause());
  await dialog.getByLabel(de?'Umdrehungen (Kamerahöhen)':'Turns (camera heights)',{exact:true}).selectOption('3');
  const shutter=dialog.getByRole('button',{name:de?'Foto aufnehmen':'Take photo',exact:true});
  let total=0;
  const shoot=async(n)=>{for(let i=0;i<n;i++){await shutter.click();total++;await dialog.locator('.capture-thumb').nth(total-1).waitFor();}};
  for(const n of [8,9,10]) {
   await shoot(n);
   if(n!==10){await dialog.getByRole('button',{name:de?'Nächste Kamerahöhe':'Next camera height'}).click();assert.equal(await shutter.isDisabled(),true);await dialog.getByRole('button',{name:de?'Fortsetzen':'Continue',exact:true}).click();}
  }
  assert.equal(await dialog.locator('.capture-zoom input').isDisabled(),true);
  const thumbnails=await dialog.locator('.capture-thumb img').evaluateAll(images=>images.map(image=>image.src));
  assert.equal(new Set(thumbnails).size,1,'identical photos still have independent selection');
  await dialog.locator('.capture-thumb input').nth(10).check();
  await dialog.getByRole('button',{name:de?'Ausgewählte löschen (1)':'Delete selected (1)',exact:true}).click();
  assert.equal(await dialog.locator('.capture-thumb').count(),26);
  // Refocusing and toggling Auto on a frozen view must not produce another photo.
  await dialog.locator('video').click();
  await dialog.locator('.capture-watch-box').waitFor();
  await dialog.locator('.capture-auto input').check();
  await dialog.getByRole('button',{name:de?'Fortsetzen':'Continue',exact:true}).click();
  await page.waitForTimeout(1800);
  assert.equal(await dialog.locator('.capture-thumb').count(),26,'changing the watched area cannot photograph the same view again');
  await dialog.locator('.capture-auto input').uncheck();

  if(process.env.STUDIO_OUT)await dialog.screenshot({path:`${process.env.STUDIO_OUT}/capture-review-${de?'de':'en'}.png`});
  await dialog.getByRole('button',{name:de?'26 Fotos verwenden':'Use 26 photos',exact:true}).click();
  await dialog.waitFor({state:'detached'});
  assert.match(await page.locator('.picked').first().innerText(),/26/);
  console.log(locale,'passed: 3 heights, arbitrary deletion, 26 photos retained');
  await context.close();
 }
}finally{await browser.close();}
