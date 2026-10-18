addon.name = 'AshitaFixture'
assert(ffxi.runtime.ecosystem == 'ashita4')
ashita.events.register('unload', 'fixture_unload', function() AshitaCore:GetChatManager():QueueCommand(1, '/echo Ashita fixture unloaded') end)
if ffxi.runtime.engine == 'luajit' then
    assert(type(jit) == 'table')
    local ffi = require('ffi')
    ffi.cdef[[size_t strlen(const char*);]]
    assert(ffi.sizeof('void*') == 4 and tonumber(ffi.C.strlen('Ashita')) == 6)
else
    assert(ffxi.runtime.engine == 'lua51' and jit == nil)
end

ashita.events.register('load', 'fixture_load', function()
    AshitaCore:GetChatManager():QueueCommand(1, '/echo Ashita fixture loaded')
end)
ashita.events.register('d3d_present', 'fixture_frame', function()
    AshitaCore:GetChatManager():QueueCommand(1, '/echo Ashita frame')
end)
