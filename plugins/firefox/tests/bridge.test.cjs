// Test the exact shipped bridge against a controlled Firefox object graph.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const script = fs.readFileSync(require('node:path').join(__dirname, '../src/bridge.js'), 'utf8');
const execute = new Function('Services', 'ChromeUtils', 'options', `return (function(){${script}}).call(null, options);`);

function fixture() {
  const frame = (id, pid, children = []) => ({id, currentWindowGlobal:{osPid:pid,innerWindowId:id*10},children});
  const tab = (id, pid, children = []) => ({label:`Tab ${id}`,linkedBrowser:{browserId:id,contentTitle:`Page ${id}`,currentURI:{spec:`https://example.test/${id}`},browsingContext:frame(id,pid,children)}});
  const a = tab(1,100,[frame(11,200)]), b = tab(2,100), c = tab(3,300), priv = tab(4,100);
  const win = (id,tabs,isPrivate=false) => ({browsingContext:{id},gBrowser:{tabs,selectedTab:tabs[0]},isPrivate,focus(){this.focused=true;}});
  const windows = [win(1,[a,b,c]),win(2,[priv],true)];
  const Services = {appinfo:{processID:42,version:'test'},wm:{getEnumerator(){return windows;}}};
  const ChromeUtils = {importESModule(){return {PrivateBrowsingUtils:{isWindowPrivate:w=>w.isPrivate}};}};
  return {run:options=>execute(Services,ChromeUtils,options),windows,a,b,c,priv};
}

test('shared processes return all related tabs and exclude private metadata',()=>{
  const f=fixture();const r=f.run({action:'resolve',pid:100});
  assert.equal(r.targets.length,2);assert.equal(r.targets[0].relationship,'main_page');
  assert.equal(f.run({action:'resolve',pid:100,includePrivate:true}).targets.length,3);
});
test('cross-origin subframes map back to their containing tab',()=>{
  const r=fixture().run({action:'resolve',pid:200});
  assert.equal(r.targets.length,1);assert.equal(r.targets[0].title,'Page 1');assert.equal(r.targets[0].relationship,'embedded_frame');
});
test('activation selects an existing tab and requests focus without adding tabs',()=>{
  const f=fixture();const target=f.run({action:'resolve',pid:100}).targets[1];
  const r=f.run({action:'activate',pid:100,target:target.id});
  assert.equal(r.activation.selected,true);assert.equal(f.windows[0].gBrowser.selectedTab,f.b);
  assert.equal(f.windows[0].focused,undefined);
  assert.equal(f.run({action:'focus',pid:100,target:target.id}).activation.focusRequested,true);
  assert.equal(f.windows[0].focused,true);assert.equal(f.windows[0].gBrowser.tabs.length,3);
});
test('closed, navigated, and migrated targets are stale',()=>{
  for(const change of [f=>{f.b.closing=true;},f=>{f.b.linkedBrowser.browsingContext.currentWindowGlobal.innerWindowId++;},f=>{f.b.linkedBrowser.browsingContext.currentWindowGlobal.osPid=999;}]){
    const f=fixture();const target=f.run({action:'resolve',pid:100}).targets[1];change(f);
    assert.equal(f.run({action:'activate',pid:100,target:target.id}).activation,null);
  }
});
test('helper processes have no fabricated tab and focus denial preserves selection',()=>{
  const f=fixture();assert.equal(f.run({action:'resolve',pid:900}).targets.length,0);
  const target=f.run({action:'resolve',pid:100}).targets[1];
  f.windows[0].focus=()=>{throw Error('denied');};
  const r=f.run({action:'activate',pid:100,target:target.id});
  assert.equal(r.activation.selected,true);assert.equal(r.activation.focusRequested,false);
  assert.equal(f.run({action:'focus',pid:100,target:target.id}).activation.focusRequested,false);
});
test('private target cannot be activated after private capability is removed',()=>{
  const f=fixture();const target=f.run({action:'resolve',pid:100,includePrivate:true}).targets[2];
  assert.equal(f.run({action:'activate',pid:100,target:target.id}).activation,null);
});
