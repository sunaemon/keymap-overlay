-- Copyright 2025 Sunaemon
-- SPDX-License-Identifier: MIT
-- keymap_overlay.lua

local MODULE = "keymap-overlay"
local log = hs.logger.new(MODULE, "info")

-- Constants for QMK Raw HID
local RAW_USAGE_PAGE = 0xFF60
local RAW_USAGE = 0x61

-- Configuration
local imageDir = hs.configdir

-- State
local devices = {}
local canvas = hs.canvas.new({ x = 0, y = 0, w = 0, h = 0 })
canvas:level("overlay")
canvas:behavior(hs.canvas.windowBehaviors.canJoinAllSpaces | hs.canvas.windowBehaviors.fullScreenAuxiliary)
canvas[1] = { type = "image", image = nil, imageAlpha = 0.85 }

local overlayVisible = false
local currentImageConfig = nil -- { id = 1, layer = 0 }

-- Helper: Load Image
local function getImage(keyboard_id, layer)
  local filename = string.format("%d_L%d.png", keyboard_id, layer)
  local path = imageDir .. "/" .. filename
  if hs.fs.attributes(path) then
    return hs.image.imageFromPath(path)
  end
  return nil
end

-- Helper: Show/Hide Overlay
local function updateOverlay(show, keyboard_id, layer)
  if show then
    local img = getImage(keyboard_id, layer)
    if img then
      local screen = hs.screen.mainScreen()
      local f = screen:frame()
      local w, h = f.w / 2, f.h / 2
      local x, y = f.x, f.y + f.h - h

      canvas:frame({ x = x, y = y, w = w, h = h })
      canvas[1].image = img
      canvas:show()
      overlayVisible = true
      currentImageConfig = { id = keyboard_id, layer = layer }
      log.i(string.format("Showing overlay: ID=%d Layer=%d", keyboard_id, layer))
    else
      log.w(string.format("Image not found for ID=%d Layer=%d", keyboard_id, layer))
      if overlayVisible then
        canvas:hide()
      end
    end
  else
    if overlayVisible then
      -- Only hide if the request matches the current shown overlay or generic hide
      if not keyboard_id or (currentImageConfig and currentImageConfig.id == keyboard_id) then
        canvas:hide()
        overlayVisible = false
        currentImageConfig = nil
        log.i("Hiding overlay")
      end
    end
  end
end

-- HID Callback
local function input_callback(device, data)
  -- Expected data format: [Command ID (0x24)] [Layer ID] [Keyboard ID (7bit) | Pressed (1bit)] [Padding...]
  -- Note: macOS HID callback string usually starts with data.

  if #data < 3 then
    return
  end

  local cmd_id = string.byte(data, 1)
  if cmd_id ~= 0x24 then
    return
  end

  local layer = string.byte(data, 2)
  local byte3 = string.byte(data, 3)

  local keyboard_id = byte3 & 0x7F
  -- Using the 8th bit for 'pressed' state (0x80) based on assumed C struct packing
  local pressed = (byte3 & 0x80) ~= 0

  -- Debug logging
  -- log.d(string.format("HID Data: %02X %02X %02X -> ID: %d, Layer: %d, Pressed: %s", cmd_id, layer, byte3, keyboard_id, layer, tostring(pressed)))

  if pressed then
    updateOverlay(true, keyboard_id, layer)
  else
    updateOverlay(false, keyboard_id, layer)
  end
end

-- Device Discovery
local function refreshDevices()
  -- Known VIDs/PIDs from repository
  -- Keyboard 1: 0x355D:0x1001 (insixty_en)
  -- Keyboard 2: 0xD010:0x1601 (kb16)
  local target_devices = {
    { vid = 0x355D, pid = 0x1001 },
    { vid = 0xD010, pid = 0x1601 },
  }

  local found_devs = hs.hid.find_devices(RAW_USAGE_PAGE, RAW_USAGE) or {}
  local new_devices = {}

  for _, dev_info in ipairs(found_devs) do
    -- Check if this device is one of our targets
    local is_target = false
    for _, t in ipairs(target_devices) do
      if dev_info:vendorID() == t.vid and dev_info:productID() == t.pid then
        is_target = true
        break
      end
    end

    if is_target then
      local path = dev_info:transport() .. "_" .. dev_info:locationID() -- Unique ID key
      if devices[path] then
        -- Keep existing device
        new_devices[path] = devices[path]
        devices[path] = nil
      else
        log.i("Found Raw HID device: " .. dev_info:productName())
        -- Note: hs.hid.new(vid, pid) might not be ideal if multiple identical devices exist,
        -- but it's what's available for creating a 'managed' HID object with callbacks.
        local dev_obj = hs.hid.new(dev_info:vendorID(), dev_info:productID())
        if dev_obj then
          dev_obj:setCallback(input_callback)
          new_devices[path] = dev_obj
          log.i("Connected to " .. dev_info:productName())
        else
          log.e("Failed to connect to " .. dev_info:productName())
        end
      end
    end
  end

  -- Cleanup remaining devices in the old table
  for path, dev_obj in pairs(devices) do
    log.i("Removing stale device: " .. path)
    dev_obj:setCallback(nil)
  end

  devices = new_devices
end

-- USB Watcher to handle plugging/unplugging
local usbWatcher = hs.usb.watcher.new(function(data)
  if data.eventType == "added" then
    hs.timer.doAfter(1.0, refreshDevices) -- Wait a bit for HID interface to be ready
  elseif data.eventType == "removed" then
    -- Cleanup is tricky without path mapping from usb watcher, but refreshing handles new connects
    -- Ideally we'd remove disconnected devices from 'devices' table but hs.hid object might handle error gracefully
    -- For simple overlay, lazy cleanup or periodic refresh is okay.
    -- Let's just trigger refresh to find new ones.
  end
end)
usbWatcher:start()

-- Initial scan
refreshDevices()

log.i("keymap-overlay: Raw HID listener started")
hs.alert.show("keymap-overlay: Raw HID listener started")

return {
  devices = devices,
  refresh = refreshDevices,
}
