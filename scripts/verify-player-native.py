"""Owned, muted native-player acceptance. No FFXI/server/retail access."""
import argparse, hashlib, json, os, subprocess, time, urllib.request, urllib.error
from pathlib import Path

def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
 ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--binaries',type=Path,required=True);ap.add_argument('--ui',type=Path,required=True);ap.add_argument('--output',type=Path,required=True);a=ap.parse_args();a.output.mkdir(parents=True,exist_ok=False)
 workspace=a.output/'workspace';(workspace/'profiles').mkdir(parents=True);(workspace/'addons/example').mkdir(parents=True)
 source=b'assert(ffxi.api_version == 1)\nffxi.events.on("load", function() ffxi.chat.queue(207,"Owned player test") end)\n'
 addon=workspace/'addons/example/example.lua';addon.write_bytes(source)
 document=dict(schema_version=1,name='Classic',width=1280,height=720,ui_scale=1.0,profile=dict(schema_version=1,root=str(workspace.resolve()),addons=[dict(name='example',ecosystem='native',engine='lua51')]))
 original=workspace/'profiles/Classic-0001.json';original.write_text(json.dumps(document),encoding='utf-8');original_hash=sha(original)
 host=a.binaries/'profile-addon-host.exe';server=a.binaries/'player_server.exe';cli=a.binaries/'windower-rs.exe'
 manifest=a.output/'manifest.json';manifest.write_text(json.dumps(dict(schema_version=1,id='example',version='1.0.0',files={'addons/example/example.lua':sha(addon)})),encoding='utf-8')
 flags=subprocess.CREATE_NO_WINDOW if os.name=='nt' else 0
 for args in [[str(cli),'package-stage',str(workspace/'packages'),str(manifest),str(workspace),str(a.output/'staged.json')]]:subprocess.run(args,check=True,capture_output=True,creationflags=flags)
 slot=json.loads((a.output/'staged.json').read_text())['slot'];selection=a.output/'slots.json';selection.write_text(json.dumps([slot]));subprocess.run([str(cli),'package-activate',str(workspace/'packages'),str(selection),str(a.output/'activated.json')],check=True,capture_output=True,creationflags=flags)
 checks=[];p=subprocess.Popen([str(server),str(workspace),str(a.ui/'dist'),'0',str(host),sha(host)],stdout=subprocess.PIPE,stderr=(a.output/'server.stderr').open('wb'),text=True,creationflags=flags)
 url=None;transcript=[]
 try:
  identity=json.loads(p.stdout.readline());assert identity['process_id']==p.pid;url='http://'+identity['address'];(a.output/'server-identity.json').write_text(json.dumps(identity,indent=2))
  def request(path,body=None,headers=None):
   h={'Origin':url,'Content-Type':'application/json','X-Signet-Request':'1'};h.update(headers or {});r=urllib.request.Request(url+path,data=None if body is None else json.dumps(body).encode(),headers=h)
   def record(status,raw):
    try:reply=json.loads(raw)
    except (ValueError,UnicodeDecodeError):reply=raw.decode('utf-8',errors='replace')
    transcript.append(dict(path=path,request=body,status=status,response=reply));return status,raw
   try:
    with urllib.request.urlopen(r,timeout=15) as response:return record(response.status,response.read())
   except urllib.error.HTTPError as e:return record(e.code,e.read())
  def rpc(action,**fields):
   status,body=request('/api/v1/rpc',dict(schema_version=1,action=action,**fields));value=json.loads(body);assert status==200,(status,value);assert value['ok'];return value['value']
  def snapshot():return json.loads(request('/api/v1/snapshot')[1])
  s=snapshot();assert len(s['profiles'])==1 and not s['game_launch']['available'];assert rpc('packages')['current']['slots']=={'example':slot};checks.append('actual native profiles and active package generation')
  for path,body,headers,expected in [('/api/v1/rpc',dict(schema_version=1,action='shutdown'),{'Origin':'https://invalid.example'},403),('/api/v1/rpc',dict(schema_version=1,action='shutdown'),{'X-Signet-Request':'0'},403),('/api/v1/snapshot',None,{'Host':'invalid.example'},403),('/../Cargo.toml',None,{},404),('/profiles/Classic-0001.json',None,{},404),('/api/v1/rpc',dict(schema_version=9,action='shutdown'),{},409),('/api/v1/rpc',dict(schema_version=1,action='shutdown',unexpected=True),{},409)]:assert request(path,body,headers)[0]==expected
  checks.append('seven HTTP origin/host/path/version/unknown-field refusals')
  browser=subprocess.run(['node',str(a.ui/'tests/native-acceptance.mjs'),url+'/',str(a.output/'browser')],cwd=a.ui,capture_output=True,text=True,creationflags=flags,timeout=120);(a.output/'browser.log').write_text(browser.stdout+browser.stderr,encoding='utf-8');assert browser.returncode==0,browser.stderr
  s=snapshot();selected=next(v for v in s['profiles'] if v['document']['name']=='Evening');running=rpc('start_offline',id=selected['id']);old=running['reply']['status']['loaded_sources']['example'];assert old==sha(addon);checks.append('owned pipe reply confirms exact source digest')
  status,_=request('/api/v1/rpc',dict(schema_version=1,action='runtime',operation='unload',name='foreign'));assert status==409;assert snapshot()['session']['process_id']==running['process_id'];checks.append('invalid control preserves owned running session')
  addon.write_bytes(b'this is broken lua !!!');reply=rpc('runtime',operation='reload',name='example');assert reply['reply']['ok'] is False;assert reply['reply']['status']['loaded_sources']['example']==old;addon.write_bytes(source);reply=rpc('runtime',operation='reload',name='example');assert reply['reply']['ok'] is True;assert reply['reply']['game_commands_sent']==0;checks.append('syntax-failed reload preserves old addon then valid reload recovers')
  rpc('stop_runtime');assert sha(original)==original_hash;assert s['profiles'][0]['document']['width']==1920;checks.append('old document unchanged and copied profile saves preserve neighbor')
  rpc('shutdown');p.wait(timeout=10);assert p.returncode==0;assert not (workspace/'.signet-player.lock').exists();checks.append('normal owned server exit releases workspace lock')
  report=dict(checks_passed=True,checks=checks,browser=json.loads((a.output/'browser/report.json').read_text()),binary_sha256={p.name:sha(p) for p in (server,host,cli)},original_profile_sha256=original_hash,game_started=False,server_services_started=False)
  (a.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8');print(json.dumps(dict(checks_passed=True,report=str(a.output/'report.json'))))
 finally:
  (a.output/'native-transcript.json').write_text(json.dumps(transcript,indent=2),encoding='utf-8')
  if p.poll() is None:
   if url:
    try:request('/api/v1/rpc',dict(schema_version=1,action='shutdown'))
    except Exception:pass
   try:p.wait(timeout=12)
   except subprocess.TimeoutExpired:p.kill();p.wait()
if __name__=='__main__':main()