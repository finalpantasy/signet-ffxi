import assert from "node:assert/strict";
import { chromium } from "playwright-core";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import axe from "axe-core";
const [url,output]=process.argv.slice(2);assert.equal(new URL(url).hostname,"127.0.0.1");await mkdir(output,{recursive:false});
const browser=await chromium.launch({headless:true,args:["--mute-audio"]});const context=await browser.newContext({viewport:{width:1100,height:850},reducedMotion:"reduce"});const page=await context.newPage();let errors=[];let external=[];page.on("pageerror",e=>errors.push(e.message));page.on("request",r=>{if(new URL(r.url()).origin!==new URL(url).origin)external.push(r.url());});const checks=[];const audits=[];
const nav=label=>page.getByRole("navigation").getByRole("button",{name:label,exact:true}).click();
try{
await page.goto(url);await page.getByText("Native workspace",{exact:true}).waitFor({timeout:10000});checks.push("native service detected without sample data");
await nav("Profiles");await page.getByLabel("Width",{exact:true}).fill("1920");await page.getByLabel("Height",{exact:true}).fill("1080");await page.getByRole("button",{name:"Save profile",exact:true}).click();await page.getByText("Saved to native profile",{exact:true}).waitFor();checks.push("native validated save");
await page.reload();await nav("Profiles");assert.equal(await page.getByLabel("Width",{exact:true}).inputValue(),"1920");checks.push("save survives browser reload");
await page.getByLabel("Copy name").fill("Evening");await page.getByRole("button",{name:"Copy profile",exact:true}).click();await page.getByText("Profile copied",{exact:true}).waitFor();checks.push("native copy");
await nav("Addons & Mods");await page.getByRole("switch",{name:"Enable example",exact:true}).click();await page.getByText("Saved to native profile",{exact:true}).waitFor();await page.getByRole("switch",{name:"Enable example",exact:true}).click();await page.getByText("Saved to native profile",{exact:true}).waitFor();checks.push("native addon selection persists and re-enables");
await nav("Play");await page.locator(".advanced > summary").click();await page.getByRole("button",{name:"Test addons",exact:true}).click();await page.getByText("Running · offline addon test",{exact:true}).waitFor();await page.getByText("Loaded in offline runtime",{exact:true}).waitFor();checks.push("real runtime autoload confirmed");
await page.getByRole("button",{name:"Unload example",exact:true}).click();await page.getByText("Not loaded",{exact:true}).waitFor();await page.getByRole("button",{name:"Load example",exact:true}).click();await page.getByText("Loaded in offline runtime",{exact:true}).waitFor();checks.push("real runtime unload and load");
await page.screenshot({path:resolve(output,"native-desktop.png")});await page.evaluate(axe.source);audits.push((await page.evaluate(()=>window.axe.run(document,{runOnly:{type:"tag",values:["wcag2a","wcag2aa","wcag21aa"]}}))).violations);
await nav("Profiles");await page.getByLabel("Width",{exact:true}).fill("800");await page.getByRole("button",{name:"Save profile",exact:true}).click();await page.getByText("Saved to native profile",{exact:true}).waitFor();await nav("Play");await page.locator(".advanced > summary").click();await page.getByText("Saved changes apply to the next session.",{exact:true}).waitFor();checks.push("running profile remains pinned after save");
await page.getByRole("button",{name:"Stop test",exact:true}).click();await page.getByText("Offline test stopped",{exact:true}).waitFor();checks.push("owned runtime shutdown");
await page.setViewportSize({width:390,height:844});await page.screenshot({path:resolve(output,"native-mobile.png"),fullPage:true});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));await page.evaluate(axe.source);audits.push((await page.evaluate(()=>window.axe.run())).violations);checks.push("narrow layout");
assert.deepEqual(errors,[]);assert.deepEqual(external,[]);assert.deepEqual(audits.flat(),[]);
await writeFile(resolve(output,"report.json"),JSON.stringify({checks_passed:true,checks,audits,errors,external,game_started:false},null,2));
}finally{await browser.close();}