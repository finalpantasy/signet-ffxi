#[test]
fn server_bridge_receipts_require_online_lab_gm_fresh_nonce_and_unique_steps() {
    let lua = mlua::Lua::new();
    lua.load(
        r#"
        output={}
        files={}
        io={open=function(path,mode)
            if mode=='rb' then return files[path] and {close=function() end} or nil end
            return {write=function(self,body) files[path]=body;return self end,close=function() return true end}
        end}
        os={rename=function(old,new) assert(files[old] and not files[new]);files[new]=files[old];files[old]=nil;return true end}
        print=function(text) output[#output+1]=text end
        p={ gm=5, zone=230, event=false }
        function p:getGMLevel() return self.gm end
        function p:getName() return 'Mithra' end
        function p:getID() return 1 end
        function p:getZoneID() return self.zone end
        function p:isInEvent() return self.event end
        online=false
        GetPlayerByName=function(name) assert(name=='Mithra');if online then return p end end
    "#,
    )
    .exec()
    .unwrap();
    let module: mlua::Table = lua
        .load(include_str!("../../lab/server/rustlab.lua"))
        .eval()
        .unwrap();
    lua.globals().set("bridge", module).unwrap();
    lua.load(r#"
        local nonce=string.rep('a',32)
        bridge.status(nonce)
        assert(output[1]:find('"online":false',1,true))
        assert(not pcall(bridge.arm,nonce))
        online=true;p.gm=1;assert(not pcall(bridge.arm,nonce))
        p.gm=5;p.event=true;assert(not pcall(bridge.arm,nonce))
        p.event=false;p.zone=1;assert(not pcall(bridge.arm,nonce))
        p.zone=230
        p.event=false;bridge.arm(nonce)
        assert(not pcall(bridge.arm,nonce))
        assert(not pcall(bridge.onTrigger,p,nonce,'wload'))
        bridge.onTrigger(p,nonce,'ready')
        local before_ready=#output
        bridge.onTrigger(p,nonce,'ready')
        assert(#output==before_ready)
        assert(not pcall(bridge.onTrigger,p,string.rep('b',32),'wload'))
        assert(not pcall(bridge.onTrigger,p,nonce,'unexpected'))
        for _,step in ipairs({'wload','aload','nload','wdelay','adelay','ndelay'}) do bridge.onTrigger(p,nonce,step) end
        bridge.snapshot(nonce)
        assert(output[#output]:find('"complete":true',1,true))
        bridge.onTrigger(p,nonce,'wload');bridge.snapshot(nonce)
        assert(output[#output]:find('"duplicates":1',1,true))
        assert(output[#output]:find('"complete":false',1,true))
        local field=string.rep('c',32)
        p.zone=102;bridge.arm(field)
        assert(output[#output]:find('"zone":102',1,true))
        p.zone=230;assert(not pcall(bridge.onTrigger,p,field,'wload'))
        p.zone=102;bridge.onTrigger(p,field,'ready');bridge.onTrigger(p,field,'wload')
        assert(output[#output]:find('"total":1',1,true))
        local reload=string.rep('d',32)
        assert(not pcall(bridge.arm,reload,'arbitrary'))
        bridge.arm(reload,'reload')
        bridge.onTrigger(p,reload,'ready')
        assert(not pcall(bridge.onTrigger,p,reload,'wload'))
        assert(not pcall(bridge.onTrigger,p,field,'reload_old1'))
        for _,step in ipairs({'reload_old1','reload_neighbor1','reload_old2','reload_new101','reload_neighbor2','reload_new102'}) do
            bridge.onTrigger(p,reload,step)
        end
        bridge.snapshot(reload)
        assert(output[#output]:find('"complete":true',1,true))
    "#).exec().unwrap();
}
