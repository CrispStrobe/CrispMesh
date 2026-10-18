// EN/DE generated controls preserve wire IDs. No reconstruction is started.
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
const browser=await chromium.launch({channel:'chrome',headless:true,args:['--enable-unsafe-webgpu','--enable-features=WebGPU',...(process.platform==='darwin'?['--use-angle=metal']:[])]});
const errors=[];
try {
 const page=await browser.newPage({locale:'en-US',viewport:{width:390,height:844}});
 page.on('pageerror',e=>errors.push(e.message));
 await page.goto((process.env.STUDIO_URL??'http://127.0.0.1:1431/')+'#/');
 const card=page.locator('section.card',{has:page.getByRole('heading',{name:/This browser|Reconstruct in this browser/})});
 await card.getByRole('link',{name:/Start in this browser|Use this browser|Open runs/}).click();
 await page.getByRole('heading',{name:'Runs',level:1}).waitFor();
 await page.getByRole('link',{name:/New run|Start the first run/}).first().click();
 const methods=page.locator('fieldset.methods');await methods.waitFor({timeout:90000});
 await page.locator('#f-method-surface').selectOption('fast');
 await page.locator('.language-choice select').selectOption('de');
 await page.getByRole('heading',{name:'Neue Rekonstruktion',exact:true}).waitFor();
 if(await page.locator('#f-method-surface').inputValue()!=='fast')throw Error('language changed quality ID');
 await methods.locator('details summary').first().click();
 const steps=methods.getByLabel(/^Optimierungsschritte/);await steps.fill('abc');
 await page.locator('form').evaluate(form=>form.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
 await page.waitForFunction(()=>document.querySelector('fieldset.methods input[aria-invalid="true"]')!==null);
 const error=await steps.locator('..').locator('.field-error').innerText();
 if(!/Eine Zahl/i.test(error))throw Error('method validation not German: '+error);
 if(await page.locator('label[for="s-sizes"]').innerText()!=='Ebenengrößen')throw Error('setting label not German');
 if(!await page.locator('#s-sizes-help').innerText().then(s=>s.includes('Längste Bildseite')))throw Error('setting description not German');
 if(process.env.STUDIO_OUT){mkdirSync(process.env.STUDIO_OUT,{recursive:true});await methods.screenshot({path:resolve(process.env.STUDIO_OUT,'methods-de.png')});await page.locator('#s-sizes').locator('xpath=ancestor::details').screenshot({path:resolve(process.env.STUDIO_OUT,'settings-de.png')});}
 await page.locator('.language-choice select').selectOption('en');
 if(await page.locator('#f-method-surface').inputValue()!=='fast')throw Error('second language switch changed quality ID');
 if(await page.locator('#s-sizes').getAttribute('id')!=='s-sizes')throw Error('language changed setting ID');
 if(errors.length)throw Error(errors.join('\n'));
 console.log('EN/DE phone settings passed: labels, descriptions, method validation, persistent engine IDs.');
} finally {await browser.close();}
