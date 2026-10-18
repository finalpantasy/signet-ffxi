-- Lua 5.1 scheduling adapters for Windower's documented coroutine extensions.
-- The native host supplies monotonic seconds; jobs run only on host ticks.
local raw_resume = coroutine.resume
local raw_status = coroutine.status
local unpack = unpack
local now, sequence = 0, 0
local jobs = {}
local closed = setmetatable({}, {__mode = 'k'})
local sleeping = {} -- private token; ordinary coroutine.yield is unaffected

local function pack(...)
    return {n = select('#', ...), ...}
end

local function delay(value)
    if type(value) ~= 'number' or value ~= value or value < 0 or value == math.huge then
        error('Delay must be a finite, nonnegative number', 3)
    end
    return value
end

local function enqueue(thread, seconds)
    if closed[thread] then return end
    sequence = sequence + 1
    jobs[thread] = {at = now + delay(seconds), order = sequence}
end

local function resume(thread, ...)
    if closed[thread] then
        return false, 'cannot resume closed coroutine'
    end
    jobs[thread] = nil
    local result = pack(raw_resume(thread, ...))
    if result[1] and raw_status(thread) == 'suspended' and result[2] == sleeping then
        enqueue(thread, result[3])
        return true
    end
    return unpack(result, 1, result.n)
end

coroutine.resume = resume
coroutine.close = function(thread)
    if type(thread) ~= 'thread' then error('Expected a coroutine', 2) end
    closed[thread] = true
    jobs[thread] = nil
end
coroutine.schedule = function(fn, seconds)
    if type(fn) ~= 'function' then error('Expected a function', 2) end
    delay(seconds)
    local thread = coroutine.create(fn)
    enqueue(thread, seconds)
    return thread
end
coroutine.sleep = function(seconds)
    delay(seconds)
    if not coroutine.running() then error('Cannot sleep outside a coroutine', 2) end
    return coroutine.yield(sleeping, seconds)
end
coroutine.wrap = function(fn)
    local thread = coroutine.create(fn)
    return function(...)
        local result = pack(resume(thread, ...))
        if not result[1] then error(result[2], 2) end
        return unpack(result, 2, result.n)
    end
end

return {
    dispatch = function(fn, ...)
        local thread = coroutine.create(fn)
        local result = pack(resume(thread, ...))
        if not result[1] then error(debug.traceback(thread, tostring(result[2])), 0) end
        return unpack(result, 2, result.n)
    end,
    tick = function(time)
        delay(time)
        if time < now then error('Scheduler clock moved backward') end
        now = time
        local due = {}
        for thread, job in pairs(jobs) do
            if job.at <= now then
                due[#due + 1] = {thread = thread, at = job.at, order = job.order}
            end
        end
        table.sort(due, function(left, right)
            return left.at < right.at or (left.at == right.at and left.order < right.order)
        end)
        local errors = {}
        for _, job in ipairs(due) do
            local current = jobs[job.thread]
            -- A previous callback can cancel/reschedule another due job.
            if current and current.order == job.order then
                jobs[job.thread] = nil
                local ok, message = resume(job.thread)
                if not ok then
                    errors[#errors + 1] = debug.traceback(job.thread, tostring(message))
                end
            end
        end
        return errors
    end,
    clear = function()
        for thread in pairs(jobs) do closed[thread] = true end
        jobs = {}
    end,
    pending = function()
        local count = 0
        for _ in pairs(jobs) do count = count + 1 end
        return count
    end,
}
