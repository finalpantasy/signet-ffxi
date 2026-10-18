-- Explicit ASCII library substitutes for the recorded replay cases, not full libraries.
local modules = {string=string, math=math, table=table, coroutine=coroutine}
local original_loaders=__replay_libraries
__replay_libraries=nil
local loading={}
local raw_error=error
local traceback=debug.traceback
local metatable_set=debug.setmetatable
local methods = {}
function T(t) return setmetatable(t or {}, {__index=methods}) end
function methods:map(fn) local out=T{}; for i,v in ipairs(self) do out[i]=fn(v) end; return out end
function methods:sconcat() return table.concat(self,' ') end
function methods:ieach(fn) for _,v in ipairs(self) do fn(v) end end
if not original_loaders.strings then
function string.startswith(s,p) return s:sub(1,#p)==p end
function string.any(s,...) for _,v in ipairs({...}) do if s==v then return true end end return false end
function string.number_or(s,d) return tonumber(s) or d end
function string.fmt(s,...) return string.format(s,...) end
function string.enclose(s,q) return q..s..q end
function string.strip_format(s) return s end
string.encoding={shift_jis='shift_jis'}
local find=string.find
function string.find(s,p,init,plain) if type(init)~='number' then init=1 end return find(s,p,init,plain) end
function string.split(s,sep,encoding,limit)
 local out=T{}; local pos=1
 while not limit or #out<limit-1 do local a,b=find(s,sep,pos,true);if not a then break end;out[#out+1]=s:sub(pos,a-1);pos=b+1 end
 out[#out+1]=s:sub(pos);return out
end
function string.args(s) local out=T{};for word in s:gmatch('%S+') do out[#out+1]=word end;return out end
end
local fm=debug.getmetatable(function()end) or {}
fm.__concat=function(a,b) return function(...) return b(a(...)) end end
debug.setmetatable(function()end,fm)
local function message(s)
 return setmetatable({value=tostring(s or '')},{__tostring=function(t)return t.value end,
 __index={append=function(t,s)t.value=t.value..tostring(s);return t end}})
end
modules.functions=true
modules.common=true
modules.chat={header=message,error=message,success=message,message=message,color1=function(_,s)return message(s) end}
local current,callback
modules.settings={load=function(default) current=current or default;return current end,
 save=function() __replay_record('settings.save','') end,
 register=function(_,_,fn)callback=fn end,
 reload=function() if callback then callback(current) end return current end,
 reset=function() if callback then callback(current) end return current end}
modules.fonts={new=__replay_font}
if __replay_resources~=nil then modules.resources=__replay_resources end
if __replay_capture_cancel then
 modules.packets={
  new=function(direction,id,fields)
   if direction~='outgoing' or id~=0x0F1 or type(fields)~='table' or type(fields.Buff)~='number' then
    return __replay_unsupported('packets.new:cancel_fixture_only')
   end
   return {direction=direction,id=id,buff=fields.Buff}
  end,
  inject=function(packet) return __replay_packet_capture(packet.direction,packet.id,packet.buff) end
 }
end
function require(name)
 if loading[name] then return raw_error('Replay library dependency cycle: '..name) end
 local loader=original_loaders[name]
 if loader then
  if loading[name] then return raw_error('Replay library dependency cycle: '..name) end
  loading[name]=true;original_loaders[name]=nil
  local ok,result=pcall(loader)
  loading[name]=nil
  if not ok then return raw_error(result) end
  modules[name]=result or (_libs and _libs[name]) or true
  __replay_record('original_library',name)
 end
 local result=modules[name]
 if result==nil then return __replay_unsupported('module:'..tostring(name)) end
 __replay_record('module',name);return result
end
-- Load pinned libraries while their standard debug metatable setup is available.
-- The addon receives only the restricted debug table below; no filesystem searchers.
io=nil;package=nil;dofile=nil;loadfile=nil;loadstring=nil;collectgarbage=nil
debug={traceback=traceback,setmetatable=function(v,mt)
 if type(v)~='function' and type(v)~='boolean' and type(v)~='number' and type(v)~='string' then return raw_error('Pinned library metatable type refused') end
 return metatable_set(v,mt)
end}
local names={};for name in pairs(original_loaders) do names[#names+1]=name end
table.sort(names)
for _,name in ipairs(names) do require(name) end
math.randomseed(123)
io=nil;debug={traceback=traceback};package=nil;dofile=nil;loadfile=nil;loadstring=nil
collectgarbage=nil
