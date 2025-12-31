-- Copyright 2025 sunaemon
-- SPDX-License-Identifier: MIT
-- keymap_overlay.lua
local MODULE = "keymap_overlay"

local function envFlag(name, default)
  local value = os.getenv(name)
  if value == nil then
    return default
  end
  value = string.lower(value)
  return value == "1" or value == "true" or value == "yes"
end

local DEBUG = envFlag("KEYMAP_OVERLAY_DEBUG", false)

local logger = hs.logger.new(MODULE, DEBUG and "debug" or "info")
local logPath = hs.configdir .. "/" .. MODULE .. ".log"

local function console(msg)
  if DEBUG then
    print(MODULE .. ": " .. msg)
  end
end

local function appendLog(line)
  local ok, f = pcall(function()
    return assert(io.open(logPath, "a"))
  end)
  if not ok or not f then
    local reason = tostring(f)
    hs.alert.show("LOG OPEN FAILED: " .. reason .. "\n" .. logPath)
    console("LOG OPEN FAILED: " .. reason .. " path=" .. logPath)
    return false
  end

  local writeOk, writeErr = pcall(function()
    f:write(os.date("%Y-%m-%d %H:%M:%S ") .. line .. "\n")
  end)
  f:close()

  if not writeOk then
    console("LOG WRITE FAILED: " .. tostring(writeErr))
    return false
  end

  return true
end

local function logI(msg)
  logger.i(msg)
  console("[I] " .. msg)
  appendLog("[I] " .. msg)
end
local function logW(msg)
  logger.w(msg)
  console("[W] " .. msg)
  appendLog("[W] " .. msg)
end
local function logE(msg)
  logger.e(msg)
  console("[E] " .. msg)
  appendLog("[E] " .. msg)
end
local function logD(msg)
  if DEBUG then
    logger.d(msg)
    console("[D] " .. msg)
    appendLog("[D] " .. msg)
  end
end

if DEBUG then
  hs.alert.show("Loaded " .. MODULE)
end
logI("Loaded " .. MODULE .. " logPath=" .. logPath)
logI("hs.configdir=" .. tostring(hs.configdir))

local imageDir = hs.configdir

local function loadKeyToLayer()
  local path = hs.configdir .. "/key-to-layer.json"
  if not hs.fs.attributes(path) then
    logE("key-to-layer.json not found; overlay disabled")
    hs.alert.show("keymap overlay disabled: key-to-layer.json missing")
    return nil
  end

  local f, err = io.open(path, "r")
  if not f then
    logE("Failed to open key-to-layer.json: " .. tostring(err))
    hs.alert.show("keymap overlay disabled: key-to-layer.json unreadable")
    return nil
  end

  local content = f:read("*a")
  f:close()

  local ok, data = pcall(hs.json.decode, content)
  if not ok or type(data) ~= "table" then
    logE("Invalid key-to-layer.json; overlay disabled")
    hs.alert.show("keymap overlay disabled: key-to-layer.json invalid")
    return nil
  end

  local mapped = {}
  for keyName, layerName in pairs(data) do
    if type(keyName) == "string" and type(layerName) == "string" then
      local keyCode = hs.keycodes.map[keyName:lower()]
      if keyCode then
        mapped[keyCode] = layerName
      else
        logW("Unknown key name in key-to-layer.json: " .. keyName)
      end
    end
  end

  if next(mapped) ~= nil then
    logI("Loaded key-to-layer.json")
    return mapped
  else
    logE("key-to-layer.json had no usable mappings; overlay disabled")
    hs.alert.show("keymap overlay disabled: no usable mappings")
    return nil
  end
end

local keyToLayer = loadKeyToLayer()

local function imagePathForLayer(layerName)
  return imageDir .. "/" .. layerName .. ".png"
end

local function loadImageOrFallback(layerName)
  local p = imagePathForLayer(layerName)
  local img = hs.image.imageFromPath(p)
  if img then
    logD("Loaded image: " .. p)
    return img
  end
  local p0 = imagePathForLayer("L0")
  local img0 = hs.image.imageFromPath(p0)
  if img0 then
    logW("Missing " .. p .. " -> fallback " .. p0)
    return img0
  end
  logE("Missing both " .. p .. " and " .. p0)
  return nil
end

local canvas = hs.canvas.new({ x = 0, y = 0, w = 0, h = 0 })
canvas:level("overlay")
pcall(function()
  canvas:behavior(hs.canvas.windowBehaviors.canJoinAllSpaces | hs.canvas.windowBehaviors.fullScreenAuxiliary)
end)

canvas[1] = { type = "image", image = nil, imageAlpha = 0.85 }

local activeKeyCode = nil
local overlayVisible = false
local lastHideAt = 0
local rearmDelay = 0.08 -- 80ms

local function showOverlay(layerName)
  local img = loadImageOrFallback(layerName)
  if not img then
    hs.alert.show("Missing keymap images: " .. layerName .. ".png (and L0.png)")
    return
  end

  local screen = hs.screen.mainScreen()
  if not screen then
    hs.alert.show("No screen detected")
    logE("No screen detected")
    return
  end

  local f = screen:frame()
  local w, h = f.w / 2, f.h / 2
  local x, y = f.x, f.y + f.h - h

  canvas:frame({ x = x, y = y, w = w, h = h })
  canvas[1].image = img
  canvas:show()
  overlayVisible = true
  logI("Overlay shown layer=" .. layerName)
end

local function hideOverlay(reason)
  lastHideAt = hs.timer.secondsSinceEpoch()
  canvas:hide()
  overlayVisible = false
  logI("Overlay hidden reason=" .. tostring(reason))
end

-- FAILSAFE
local failsafeSeconds = 0.35
local hideTimer = nil
local lastBumpAt = 0
local bumpMinInterval = 0.05 -- 50ms（repeatでのタイマー作り直しを減らす）

local function bumpFailsafe(reason)
  local now = hs.timer.secondsSinceEpoch()
  if now - lastBumpAt < bumpMinInterval then
    return
  end
  lastBumpAt = now

  if hideTimer then
    hideTimer:stop()
  end
  hideTimer = hs.timer.doAfter(failsafeSeconds, function()
    logW("Failsafe hide fired (" .. tostring(reason) .. ")")
    hideOverlay("failsafe")
    activeKeyCode = nil
    hideTimer = nil
  end)
end

local tap = hs.eventtap.new({ hs.eventtap.event.types.keyDown, hs.eventtap.event.types.keyUp }, function(event)
  local ok, ret = xpcall(function()
    if not keyToLayer then
      return false
    end

    local keyCode = event:getKeyCode()
    local layerName = keyToLayer[keyCode]
    if not layerName then
      return false
    end

    local t = event:getType()
    local isRepeat = (event:getProperty(hs.eventtap.event.properties.keyboardEventAutorepeat) == 1)

    bumpFailsafe(isRepeat and "repeat" or "edge")

    if not isRepeat then
      logI("Key event: " .. tostring(t) .. " keyCode " .. tostring(keyCode) .. " layer " .. tostring(layerName))
    end

    if t == hs.eventtap.event.types.keyDown then
      local now = hs.timer.secondsSinceEpoch()
      if now - lastHideAt < rearmDelay then
        return true
      end
      if activeKeyCode ~= keyCode or not overlayVisible then
        activeKeyCode = keyCode
        showOverlay(layerName)
      end
    elseif t == hs.eventtap.event.types.keyUp then
      hideOverlay("keyUp")
      activeKeyCode = nil
      if hideTimer then
        hideTimer:stop()
        hideTimer = nil
      end
    end

    return true
  end, function(err)
    local trace = debug.traceback(err, 2)
    hs.alert.show("eventtap error; see log")
    logE("eventtap ERROR:\n" .. trace)
    return false
  end)

  if not ok then
    hs.alert.show("xpcall failed; see log")
    logE("xpcall failed ret=" .. tostring(ret))
    hideOverlay("exception")
    activeKeyCode = nil
    return false
  end
  return ret
end)

local started = tap:start()
logI("eventtap started=" .. tostring(started))
if not started then
  hs.alert.show("eventtap failed: enable Accessibility for Hammerspoon")
  logE("eventtap failed to start (Accessibility?)")
end

hs.timer.doEvery(2, function()
  if tap and not tap:isEnabled() then
    hs.alert.show("eventtap restarted")
    logW("eventtap disabled -> restarting")
    tap:start()
  end
end)

hs.hotkey.bind({ "cmd", "alt", "ctrl" }, "F12", function()
  logI(
    "manual dump: activeKeyCode="
      .. tostring(activeKeyCode)
      .. " overlayVisible="
      .. tostring(overlayVisible)
      .. " canvasShowing="
      .. tostring(canvas:isShowing())
  )
end)
