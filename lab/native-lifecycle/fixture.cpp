// Controlled x86 plugin fixtures, never installed in FFXI.
// Ashita prefixes match the pinned LGPL SDK 4.30, Ashita.h:1422/1845/1915.
// Windower's first four explicit-self stdcall slots follow docs/plugin-abi.md.
// Windower callbacks below use a private fixture export: unknown legacy slots are NOT guessed.
#include <stdint.h>
#include <string.h>
#include <new>
static uint32_t counts[16] = {};
extern "C" uint32_t __stdcall FixtureProtocol() { return 0x52534631; }
extern "C" uint32_t __stdcall FixtureCount(uint32_t i) { return i < 16 ? counts[i] : 0xffffffff; }
struct Capture { int (__cdecl *queue)(void*,int,const char*); void* context; };
struct WObject;
struct WBoundary {
 const char* (__stdcall *author)(WObject*);
 const char* (__stdcall *name)(WObject*);
 void (__stdcall *initialize)(WObject*,void*);
 void (__stdcall *destroy)(WObject*);
};
struct WObject { WBoundary* table; Capture* capture; bool active; };
static const char* __stdcall wauthor(WObject*) {return "Rust offline ABI fixture";}
static const char* __stdcall wname(WObject*) {return "WindowerLifecycleFixture";}
static void __stdcall winit(WObject* self,void* host) {
 if (!self || self->active || !host) {++counts[15];return;}
 auto table=*reinterpret_cast<uintptr_t**>(host);
 auto getter=reinterpret_cast<void*(__stdcall *)(void*)>(table[3]);
 self->capture=reinterpret_cast<Capture*>(getter(host));
 if (!self->capture || !self->capture->queue) {++counts[15];return;}
 self->active=true;++counts[1];self->capture->queue(self->capture->context,7,"/echo W fixture init");
}
static void __stdcall wdestroy(WObject* self) {if(self){if(self->active)++counts[2];self->active=false;delete self;++counts[3];}}
static WBoundary wtable={wauthor,wname,winit,wdestroy};
extern "C" uint32_t __stdcall GetInterfaceVersion() {return 0x04070300;}
extern "C" WObject* __stdcall CreateInstance() {++counts[0];return new(std::nothrow) WObject{&wtable,nullptr,false};}
extern "C" bool __stdcall FixtureWindowerCommand(WObject* self,int mode,const char* command,bool blocked) {
 if(!self || !self->active || !command){++counts[15];return blocked;}
 ++counts[4];if(blocked || strcmp(command,"/fixture block")==0)return true;
 self->capture->queue(self->capture->context,mode,command);return false;
}

// Source-confirmed C++ virtual prefixes. No virtual destructor is inserted.
struct IChatPrefix {virtual void ParseCommand(int32_t,const char*)=0;virtual void QueueCommand(int32_t,const char*)=0;};
struct ICorePrefix {
 virtual void* GetHandle() const=0;virtual const char* GetInstallPath() const=0;
 virtual void* GetDirect3DDevice() const=0;virtual void* GetProperties() const=0;
 virtual IChatPrefix* GetChatManager() const=0;
};
struct IPluginPrefix {
 virtual const char* GetName() const=0;virtual const char* GetAuthor() const=0;
 virtual const char* GetDescription() const=0;virtual const char* GetLink() const=0;
 virtual double GetVersion() const=0;virtual double GetInterfaceVersion() const=0;
 virtual int32_t GetPriority() const=0;virtual uint32_t GetFlags() const=0;
 virtual bool Initialize(ICorePrefix*,void*,uint32_t)=0;virtual void Release()=0;
 virtual void HandleEvent(const char*,const void*,uint32_t)=0;
 virtual bool HandleCommand(int32_t,const char*,bool)=0;
};
struct AObject final:IPluginPrefix {
 ICorePrefix* core=nullptr;bool active=false;
 const char* GetName() const override{return "AshitaLifecycleFixture";}
 const char* GetAuthor() const override{return "Rust offline ABI fixture";}
 const char* GetDescription() const override{return "Controlled mock services only";}
 const char* GetLink() const override{return "local-fixture";}
 double GetVersion() const override{return 1.0;}
 double GetInterfaceVersion() const override{return 4.30;}
 int32_t GetPriority() const override{return 0;}
 uint32_t GetFlags() const override{return 1;}
 bool Initialize(ICorePrefix* host,void*,uint32_t id) override {
  ++counts[6];if(id==0 || !host || active)return false;
  if(strcmp(host->GetInstallPath(),"RustOffline")!=0 || !host->GetChatManager())return false;
  core=host;active=true;++counts[7];core->GetChatManager()->QueueCommand(3,"/echo A fixture init");return true;
 }
 void Release() override {if(active){active=false;++counts[8];}else ++counts[15];}
 void HandleEvent(const char* name,const void* data,uint32_t size) override {
  if(active && name && strcmp(name,"fixture")==0 && size==3 && data && static_cast<const uint8_t*>(data)[2]==3)++counts[10];else ++counts[15];
 }
 bool HandleCommand(int32_t mode,const char* command,bool blocked) override {
  if(!active || !command){++counts[15];return blocked;}
  ++counts[11];if(blocked || strcmp(command,"/fixture block")==0)return true;
  core->GetChatManager()->QueueCommand(mode,command);return false;
 }
 ~AObject(){++counts[9];}
};
extern "C" double __stdcall expGetInterfaceVersion(){return 4.30;}
extern "C" IPluginPrefix* __stdcall expCreatePlugin(const char*){++counts[5];return new(std::nothrow) AObject();}
extern "C" void __stdcall expDestroyPlugin(IPluginPrefix* object){delete static_cast<AObject*>(object);}
