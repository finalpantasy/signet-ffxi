_addon.name = 'RustFixture'
_addon.version = '1.0'
windower.register_event('unload', function() windower.send_command('Windower fixture unloaded') end)

local frames = 0
windower.register_event('load', function()
    windower.add_to_chat(207, 'Rust Lua host loaded')
    coroutine.schedule(function()
        windower.send_command('scheduled ' .. require('native_fixture').message)
        coroutine.sleep(0)
        windower.send_command('scheduled coroutine resumed')
    end, 0)
end)

windower.register_event('keyboard', function(dik, pressed, flags, blocked)
    assert(dik == 30 and pressed and flags == 0 and not blocked)
    windower.send_command('echo Rust keyboard')
    return true
end)

windower.register_event('mouse', function(kind, x, y, delta, blocked)
    assert(kind == 1 and x == 4 and y == 7 and delta == 0 and not blocked)
    windower.add_to_chat(207, 'Rust mouse')
    return true
end)

windower.register_event('prerender', function()
    frames = frames + 1
    windower.send_command('echo Rust frame ' .. frames)
end)
