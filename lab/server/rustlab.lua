-- Rust integration lab: explicit nonce-scoped command receipts only.
-- Loaded through the existing loopback controller; no persistent character edits.
local bridge = { cmdprops = { permission = 5, parameters = 'ss' } }
local sessions = {}
local count = 0
local sequences = {}
local directory = 'F:/FFXI-Modding/LocalServer/private/rust-bridge-receipts/'
local expected = { wload=true, wdelay=true, aload=true, adelay=true, nload=true, ndelay=true }
local reload_expected = { reload_old1=true, reload_neighbor1=true, reload_old2=true,
    reload_new101=true, reload_neighbor2=true, reload_new102=true }
local allowed_zones = { [102]=true, [230]=true }
local function nonce_ok(nonce)
    return type(nonce)=='string' and #nonce==32 and nonce:match('^[0-9a-f]+$') ~= nil
end
local function player()
    local p=GetPlayerByName('Mithra')
    if not p or p:getGMLevel()<5 then return nil end
    return p
end
local function emit(nonce, body)
    assert(nonce_ok(nonce), 'Invalid Rust lab nonce')
    local sequence=(sequences[nonce] or 0)+1
    assert(sequence<=256,'Rust lab receipt limit reached')
    sequences[nonce]=sequence
    body='{"nonce":"'..nonce..'","sequence":'..sequence..','..body:sub(2)
    local path=directory..nonce..'-'..string.format('%06d',sequence)..'.json'
    local existing=io.open(path,'rb')
    if existing then existing:close();error('Rust lab receipt already exists') end
    local file=assert(io.open(path..'.tmp','wb'),'Cannot create Rust lab receipt')
    assert(file:write(body));assert(file:close())
    assert(os.rename(path..'.tmp',path),'Cannot publish completed Rust lab receipt')
    print('RUSTLAB:' .. nonce .. ':' .. body)
end
function bridge.status(nonce)
    local p=player()
    local online=p~=nil
    local zone=online and p:getZoneID() or 0
    local event=online and p:isInEvent() or false
    emit(nonce,string.format('{"kind":"status","online":%s,"zone":%d,"in_event":%s}',tostring(online),zone,tostring(event)))
end
function bridge.arm(nonce,scenario)
    assert(nonce_ok(nonce), 'Invalid Rust lab nonce')
    assert(not sessions[nonce], 'Rust lab nonce already used')
    scenario=scenario or 'persistent'
    assert(scenario=='persistent' or scenario=='reload','Unknown Rust lab scenario')
    local p=assert(player(),'Rust lab character must be online and GM level 5')
    assert(allowed_zones[p:getZoneID()] and not p:isInEvent(),'Rust lab requires idle La Theine Plateau or Southern San d Oria')
    assert(count<32,'Restart the Rust lab module before more trials')
    sessions[nonce]={ id=p:getID(), zone=p:getZoneID(), scenario=scenario,
        expected=scenario=='reload' and reload_expected or expected, ready=false, steps={}, total=0, duplicates=0 }
    count=count+1
    emit(nonce,'{"kind":"armed","zone":'..p:getZoneID()..'}')
end
function bridge.onTrigger(p,nonce,step)
    assert(nonce_ok(nonce), 'Invalid Rust lab nonce')
    local session=assert(sessions[nonce],'Rust lab session is not armed')
    assert(p:getName()=='Mithra' and p:getID()==session.id and p:getGMLevel()>=5,'Wrong lab character')
    assert(p:getZoneID()==session.zone and not p:isInEvent(),'Rust lab character left the test state')
    if step=='ready' then
        if not session.ready then
            session.ready=true
            emit(nonce,'{"kind":"ready","zone":'..p:getZoneID()..'}')
        end
        return
    end
    assert(session.ready,'Client world readiness was not confirmed')
    assert(session.expected[step],'Unknown Rust lab step')
    if session.steps[step] then
        session.duplicates=session.duplicates+1
        emit(nonce,'{"kind":"duplicate","step":"'..step..'"}')
        return
    end
    session.steps[step]=true
    session.total=session.total+1
    emit(nonce,'{"kind":"receipt","step":"'..step..'","total":'..session.total..'}')
end
function bridge.snapshot(nonce)
    local session=assert(sessions[nonce],'Unknown Rust lab session')
    emit(nonce,string.format('{"kind":"snapshot","received":%d,"duplicates":%d,"complete":%s}',session.total,session.duplicates,tostring(session.ready and session.total==6 and session.duplicates==0)))
end
return bridge
