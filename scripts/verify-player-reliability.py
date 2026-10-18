"""Focused real-server review regressions. Starts only owned fixture processes."""
from pathlib import Path
import argparse,json,subprocess,hashlib,os,urllib.request,urllib.error,ctypes
p=argparse.ArgumentParser();p.add_argument('--binaries',type=Path,required=True);p.add_argument('--ui',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--scenario',choices=['stale','death','browser','all'],default='all');a=p.parse_args();a.output.mkdir(parents=True,exist_ok=False)
w=a.output/'workspace';(w/'profiles').mkdir(parents=True);(w/'addons/example').mkdir(parents=True);(w/'addons/example/example.lua').write_text('return 1',encoding='utf-8');doc=dict(schema_version=1,name='Classic',width=1280,height=720,ui_scale=1.0,profile=dict(schema_version=1,root=str(w),addons=[dict(name='example',ecosystem='native',engine='lua51')]));(w/'profiles/Classic-0001.json').write_text(json.dumps(doc),encoding='utf-8');host=a.binaries/'profile-addon-host.exe';flags=subprocess.CREATE_NO_WINDOW;sha=lambda f:hashlib.sha256(f.read_bytes()).hexdigest();proc=subprocess.Popen([str(a.binaries/'player_server.exe'),str(w),str(a.ui/'dist'),'0',str(host),sha(host)],stdout=subprocess.PIPE,stderr=(a.output/'server.stderr').open('wb'),text=True,creationflags=flags);identity=json.loads(proc.stdout.readline());url='http://'+identity['address'];trace=[]
def request(action,**fields):
 body=dict(schema_version=1,action=action,**fields);r=urllib.request.Request(url+'/api/v1/rpc',data=json.dumps(body).encode(),headers={'Origin':url,'Content-Type':'application/json','X-Signet-Request':'1'})
 try:
  with urllib.request.urlopen(r,timeout=15) as reply:status=reply.status;value=json.loads(reply.read())
 except urllib.error.HTTPError as e:status=e.code;value=json.loads(e.read())
 trace.append(dict(request=body,status=status,response=value));return status,value
def snapshot():
 with urllib.request.urlopen(url+'/api/v1/snapshot',timeout=15) as r:return json.loads(r.read())
try:
 if a.scenario in ['stale','all']:
  old=snapshot()['profiles'][0]['id'];status,value=request('profile',request=dict(schema_version=1,action='save_profile',id=old,name='Classic',width=1280,height=720,ui_scale=1.0,enabled=['example']));assert status==200
  status,value=request('start_offline',id=old);assert status==409,(status,value);assert snapshot()['session'] is None
 if a.scenario in ['browser','all']:
  r=subprocess.run(['node',str(a.ui/'tests/native-reliability.mjs'),url+'/',str(a.output/'browser')],cwd=a.ui,capture_output=True,text=True,timeout=120,creationflags=flags);(a.output/'browser.log').write_text(r.stdout+r.stderr,encoding='utf-8');assert r.returncode==0,r.stderr
 if a.scenario in ['death','all']:
  status,value=request('start_offline',id=snapshot()['profiles'][0]['id']);assert status==200;pid=value['value']['process_id']
  from ctypes import wintypes as t
  k=ctypes.WinDLL('kernel32',use_last_error=True);k.OpenProcess.argtypes=[t.DWORD,t.BOOL,t.DWORD];k.OpenProcess.restype=t.HANDLE;k.QueryFullProcessImageNameW.argtypes=[t.HANDLE,t.DWORD,t.LPWSTR,ctypes.POINTER(t.DWORD)];k.TerminateProcess.argtypes=[t.HANDLE,t.UINT];k.WaitForSingleObject.argtypes=[t.HANDLE,t.DWORD];k.CloseHandle.argtypes=[t.HANDLE];handle=k.OpenProcess(0x101001,False,pid);assert handle
  try:
   buffer=ctypes.create_unicode_buffer(32768);length=t.DWORD(len(buffer));assert k.QueryFullProcessImageNameW(handle,0,buffer,ctypes.byref(length));assert Path(buffer.value).resolve()==host.resolve(),'Refuse unrelated process';assert k.TerminateProcess(handle,73);assert k.WaitForSingleObject(handle,5000)==0
  finally:k.CloseHandle(handle)
  state=snapshot();assert state['session'] is None,state
  status,value=request('start_offline',id=state['profiles'][0]['id']);assert status==200;status,value=request('stop_runtime');assert status==200;assert value['value']['exit_code']==0,value
 request('shutdown');proc.wait(timeout=10);assert proc.returncode==0;(a.output/'report.json').write_text(json.dumps(dict(checks_passed=True,scenario=a.scenario,server_identity=identity,trace=trace),indent=2),encoding='utf-8');print(json.dumps(dict(checks_passed=True,scenario=a.scenario)))
finally:
 if proc.poll() is None:
  try:request('shutdown');proc.wait(timeout=10)
  except Exception:proc.kill();proc.wait()
 (a.output/'trace.json').write_text(json.dumps(trace,indent=2),encoding='utf-8')