import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Pango from 'gi://Pango';
import St from 'gi://St';

import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const BUS_NAME = 'com.sunaemon.KeymapOverlay';
const OBJECT_PATH = '/com/sunaemon/KeymapOverlay';
const RENDERER_INTERFACE = 'com.sunaemon.KeymapOverlay.Renderer1';
const INTERFACE_SCHEMA = 'org.gnome.desktop.interface';
const DEFAULT_PREFERENCES = {
  position: 'center',
  opacity_percent: 100,
  scale_percent: 100,
};
const RENDERER_XML = `
<node>
  <interface name="${RENDERER_INTERFACE}">
    <method name="GetState">
      <arg type="t" name="generation" direction="out"/>
      <arg type="b" name="visible" direction="out"/>
      <arg type="s" name="model_json" direction="out"/>
    </method>
    <signal name="StateChanged">
      <arg type="t" name="generation"/>
      <arg type="b" name="visible"/>
      <arg type="s" name="model_json"/>
    </signal>
  </interface>
</node>`;
const RendererProxy = Gio.DBusProxy.makeProxyWrapper(RENDERER_XML);

export default class KeymapOverlayExtension extends Extension {
  enable() {
    this._enabled = true;
    this._generation = -1;
    this._overlay = null;
    this._preferences = this._loadPreferences();
    this._buildIndicator();
    this._watchPreferences();
    this._interfaceSettings = new Gio.Settings({ schema_id: INTERFACE_SCHEMA });
    this._colorSchemeId = this._interfaceSettings.connect(
      'changed::color-scheme',
      () => this._syncColorScheme()
    );
    this._proxy = new RendererProxy(
      Gio.DBus.session,
      BUS_NAME,
      OBJECT_PATH,
      (proxy, error) => {
        if (!this._enabled) return;
        if (error) {
          console.error(`Keymap Overlay: ${error.message}`);
          return;
        }
        this._proxySignalId = proxy.connectSignal(
          'StateChanged',
          (_proxy, _sender, [generation, visible, modelJson]) =>
            this._applyState(generation, visible, modelJson)
        );
        this._ownerSignalId = proxy.connect('notify::g-name-owner', () => {
          this._generation = -1;
          if (proxy.g_name_owner) this._refreshState();
          else {
            this._lastVisible = false;
            this._lastModelJson = '';
            this._hide();
          }
        });
        if (proxy.g_name_owner) this._refreshState();
      },
      null,
      Gio.DBusProxyFlags.DO_NOT_AUTO_START
    );
  }

  disable() {
    this._enabled = false;
    if (this._proxy && this._proxySignalId)
      this._proxy.disconnectSignal(this._proxySignalId);
    if (this._proxy && this._ownerSignalId)
      this._proxy.disconnect(this._ownerSignalId);
    this._proxy = null;
    this._proxySignalId = 0;
    this._ownerSignalId = 0;
    if (this._interfaceSettings && this._colorSchemeId)
      this._interfaceSettings.disconnect(this._colorSchemeId);
    this._interfaceSettings = null;
    this._colorSchemeId = 0;
    this._destroyOverlay();
    if (this._preferencesMonitor) this._preferencesMonitor.cancel();
    this._preferencesMonitor = null;
    if (this._indicator) this._indicator.destroy();
    this._indicator = null;
  }

  _refreshState() {
    this._proxy.GetStateRemote((result, error) => {
      if (!this._enabled) return;
      if (error) {
        console.error(`Keymap Overlay: ${error.message}`);
        this._hide();
        return;
      }
      this._applyState(...result);
    });
  }

  _applyState(generation, visible, modelJson) {
    generation = Number(generation);
    if (generation <= this._generation) return;
    this._generation = generation;
    this._lastVisible = visible;
    this._lastModelJson = modelJson;
    if (!visible) {
      this._hide();
      return;
    }

    let model;
    try {
      model = JSON.parse(modelJson);
      if (
        model.version !== 2 ||
        model.width <= 0 ||
        model.height <= 0 ||
        !Array.isArray(model.keys) ||
        !Array.isArray(model.encoders)
      )
        throw new Error('invalid model header');
    } catch (error) {
      console.error(`Keymap Overlay: cannot decode model: ${error.message}`);
      this._hide();
      return;
    }
    this._render(model);
  }

  _render(model) {
    this._destroyOverlay();
    const horizontalPadding = model.encoders.length
      ? Math.ceil(
          Math.max(...model.encoders.map((encoder) => encoder.size)) * 0.25
        )
      : 0;
    const overlay = new St.Widget({
      style_class: this._overlayStyleClass(),
      reactive: false,
      can_focus: false,
      clip_to_allocation: true,
      visible: false,
    });
    overlay.set_size(model.width + horizontalPadding * 2, model.height);
    overlay.set_scale(
      this._preferences.scale_percent / 100,
      this._preferences.scale_percent / 100
    );
    overlay.opacity = Math.round(
      (this._preferences.opacity_percent / 100) * 255
    );
    const content = new St.Widget({ reactive: false, can_focus: false });
    content.set_position(horizontalPadding, 0);
    content.set_size(model.width, model.height);
    content.add_child(
      this._label(`L${model.layer}`, 20, 20, 60, model.header_font_size)
    );
    for (const key of model.keys) content.add_child(this._key(key, model));
    for (const encoder of model.encoders)
      content.add_child(this._encoder(encoder, model));
    overlay.add_child(content);

    Main.layoutManager.addTopChrome(overlay, {
      affectsStruts: false,
      trackFullscreen: true,
    });
    const monitor =
      Main.layoutManager.currentMonitor ?? Main.layoutManager.primaryMonitor;
    const scaledWidth = overlay.width * overlay.scale_x;
    const scaledHeight = overlay.height * overlay.scale_y;
    let y = monitor.y + Math.round((monitor.height - scaledHeight) / 2);
    if (this._preferences.position === 'top') y = monitor.y;
    if (this._preferences.position === 'bottom')
      y = monitor.y + monitor.height - scaledHeight;
    overlay.set_position(
      monitor.x + Math.round((monitor.width - scaledWidth) / 2),
      y
    );
    overlay.show();
    this._overlay = overlay;
  }

  _key(key, model) {
    const actor = new St.Widget({
      style_class: 'button keymap-overlay-key',
      reactive: false,
      can_focus: false,
    });
    actor.set_position(key.x, key.y);
    actor.set_size(key.width, key.height);
    if (key.held) actor.add_style_pseudo_class('checked');
    actor.add_child(
      this._label(
        key.label.join('\n'),
        4,
        0,
        key.width - 8,
        model.key_font_size,
        key.height
      )
    );
    return actor;
  }

  _encoder(encoder, model) {
    const group = new St.Widget({ reactive: false, can_focus: false });
    group.set_position(encoder.x, encoder.y);
    group.set_size(encoder.size, encoder.size);
    const halfSize = encoder.size / 2;
    const labelGap = 3;
    const labelWidth = encoder.size * 0.75 - labelGap;

    const dial = new St.Widget({
      style_class: 'button keymap-overlay-encoder',
      reactive: false,
      can_focus: false,
    });
    dial.set_size(encoder.size, encoder.size);
    if (encoder.held) dial.add_style_pseudo_class('checked');
    dial.add_child(
      this._label(
        encoder.press ? `P ${encoder.press}` : '',
        4,
        0,
        encoder.size - 8,
        model.encoder_font_size,
        encoder.size
      )
    );
    group.add_child(dial);
    group.add_child(
      this._label(
        encoder.counter_clockwise.length
          ? `← ${this._compactEncoderActions(encoder.counter_clockwise)}`
          : '',
        halfSize - encoder.size * 0.75,
        -model.encoder_font_size * 2,
        labelWidth,
        model.encoder_font_size,
        model.encoder_font_size * 2,
        true,
        Pango.Alignment.RIGHT
      )
    );
    group.add_child(
      this._label(
        encoder.clockwise.length
          ? `${this._compactEncoderActions(encoder.clockwise)} →`
          : '',
        halfSize - labelGap,
        -model.encoder_font_size * 2,
        labelWidth,
        model.encoder_font_size,
        model.encoder_font_size * 2,
        true,
        Pango.Alignment.LEFT
      )
    );
    return group;
  }

  _compactEncoderActions(actions) {
    return actions
      .map((action) => action.replace(/^BRI\s*/, 'B').replace(/^VOL\s*/, 'V'))
      .join(' ');
  }

  _label(
    text,
    x,
    y,
    width,
    fontSize,
    height = fontSize * 2,
    singleLine = false,
    alignment = Pango.Alignment.CENTER
  ) {
    const textAlign =
      alignment === Pango.Alignment.LEFT
        ? 'left'
        : alignment === Pango.Alignment.RIGHT
          ? 'right'
          : 'center';
    const label = new St.Label({
      text,
      style_class: 'keymap-overlay-label',
      style: `font-size: ${fontSize}px; text-align: ${textAlign};`,
      x_align:
        alignment === Pango.Alignment.LEFT
          ? Clutter.ActorAlign.START
          : alignment === Pango.Alignment.RIGHT
            ? Clutter.ActorAlign.END
            : Clutter.ActorAlign.CENTER,
      y_align: Clutter.ActorAlign.CENTER,
    });
    label.clutter_text.set_line_wrap(!singleLine);
    label.clutter_text.set_line_alignment(alignment);
    label.clutter_text.set_ellipsize(
      singleLine ? Pango.EllipsizeMode.END : Pango.EllipsizeMode.NONE
    );
    const container = new St.Bin({
      child: label,
      reactive: false,
      can_focus: false,
      clip_to_allocation: true,
    });
    container.set_position(x, y);
    container.set_size(Math.max(1, width), Math.max(1, height));
    return container;
  }

  _hide() {
    this._destroyOverlay();
  }

  _preferencesFile() {
    return Gio.File.new_for_path(
      GLib.build_filenamev([
        GLib.get_user_config_dir(),
        'keymap-overlay',
        'preferences.json',
      ])
    );
  }

  _loadPreferences() {
    try {
      const [, contents] = this._preferencesFile().load_contents(null);
      const parsed = JSON.parse(new TextDecoder().decode(contents));
      if (
        parsed === null ||
        Array.isArray(parsed) ||
        typeof parsed !== 'object'
      )
        throw new Error('preferences must be a JSON object');
      const allowed = new Set([
        'position',
        'opacity_percent',
        'scale_percent',
        'enabled',
      ]);
      for (const key of Object.keys(parsed))
        if (!allowed.has(key)) throw new Error(`unknown preference: ${key}`);
      if (
        'position' in parsed &&
        (typeof parsed.position !== 'string' ||
          !['top', 'center', 'bottom'].includes(parsed.position))
      )
        throw new Error('position must be top, center, or bottom');
      if (
        'opacity_percent' in parsed &&
        (typeof parsed.opacity_percent !== 'number' ||
          ![50, 75, 90, 100].includes(parsed.opacity_percent))
      )
        throw new Error('opacity_percent must be 50, 75, 90, or 100');
      if (
        'scale_percent' in parsed &&
        (typeof parsed.scale_percent !== 'number' ||
          ![75, 100, 125, 150].includes(parsed.scale_percent))
      )
        throw new Error('scale_percent must be 75, 100, 125, or 150');
      delete parsed.enabled;
      return { ...DEFAULT_PREFERENCES, ...parsed };
    } catch (error) {
      if (!error.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND))
        console.error(
          `Keymap Overlay: cannot read preferences: ${error.message}`
        );
      return { ...DEFAULT_PREFERENCES };
    }
  }

  _savePreferences() {
    const file = this._preferencesFile();
    GLib.mkdir_with_parents(file.get_parent().get_path(), 0o700);
    file.replace_contents(
      new TextEncoder().encode(JSON.stringify(this._preferences, null, 2)),
      null,
      false,
      Gio.FileCreateFlags.REPLACE_DESTINATION,
      null
    );
  }

  _watchPreferences() {
    try {
      const directory = this._preferencesFile().get_parent();
      GLib.mkdir_with_parents(directory.get_path(), 0o700);
      this._preferencesMonitor = directory.monitor_directory(
        Gio.FileMonitorFlags.NONE,
        null
      );
      this._preferencesMonitor.connect('changed', (_monitor, file) => {
        if (file.get_basename() !== 'preferences.json') return;
        this._preferences = this._loadPreferences();
        this._syncIndicator();
        this._refreshVisibleOverlay();
      });
    } catch (error) {
      console.error(
        `Keymap Overlay: cannot watch preferences: ${error.message}`
      );
    }
  }

  _buildIndicator() {
    this._indicator = new PanelMenu.Button(0, 'Keymap Overlay', false);
    this._indicator.add_child(
      new St.Icon({
        icon_name: 'input-keyboard-symbolic',
        style_class: 'system-status-icon',
      })
    );
    this._launchAtLoginItem = new PopupMenu.PopupSwitchMenuItem(
      'Launch at Login',
      this._launchAtLoginEnabled()
    );
    this._launchAtLoginItem.connect('toggled', (_item, enabled) =>
      this._setLaunchAtLogin(enabled)
    );
    this._indicator.menu.addMenuItem(this._launchAtLoginItem);
    this._positionItems = this._addChoiceMenu(
      'Position',
      [
        ['Top', 'top'],
        ['Center', 'center'],
        ['Bottom', 'bottom'],
      ],
      (value) => this._updatePreferences({ position: value })
    );
    this._opacityItems = this._addChoiceMenu(
      'Opacity',
      [50, 75, 90, 100].map((value) => [`${value}%`, value]),
      (value) => this._updatePreferences({ opacity_percent: value })
    );
    this._scaleItems = this._addChoiceMenu(
      'Scale',
      [75, 100, 125, 150].map((value) => [`${value}%`, value]),
      (value) => this._updatePreferences({ scale_percent: value })
    );
    this._indicator.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
    const reload = new PopupMenu.PopupMenuItem('Reload Keyboards');
    reload.connect('activate', () =>
      Gio.DBus.session.call(
        BUS_NAME,
        OBJECT_PATH,
        RENDERER_INTERFACE,
        'ReloadKeyboards',
        null,
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
        null
      )
    );
    this._indicator.menu.addMenuItem(reload);
    const version = new PopupMenu.PopupMenuItem(
      `Keymap Overlay ${this.metadata['version-name'] ?? this.metadata.version}`,
      {
        reactive: false,
      }
    );
    this._indicator.menu.addMenuItem(version);
    const quit = new PopupMenu.PopupMenuItem('Quit');
    quit.connect('activate', () => {
      Gio.DBus.session.call(
        'org.freedesktop.systemd1',
        '/org/freedesktop/systemd1',
        'org.freedesktop.systemd1.Manager',
        'StopUnit',
        new GLib.Variant('(ss)', ['keymap-overlay.service', 'replace']),
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
        null
      );
      this.disable();
    });
    this._indicator.menu.addMenuItem(quit);
    Main.panel.addToStatusArea('keymap-overlay', this._indicator);
    this._syncIndicator();
  }

  _addChoiceMenu(title, choices, activate) {
    const submenu = new PopupMenu.PopupSubMenuMenuItem(title);
    const items = new Map();
    for (const [label, value] of choices) {
      const item = new PopupMenu.PopupMenuItem(label);
      item.connect('activate', () => activate(value));
      submenu.menu.addMenuItem(item);
      items.set(value, item);
    }
    this._indicator.menu.addMenuItem(submenu);
    return items;
  }

  _syncIndicator() {
    this._syncChoice(this._positionItems, this._preferences.position);
    this._syncChoice(this._opacityItems, this._preferences.opacity_percent);
    this._syncChoice(this._scaleItems, this._preferences.scale_percent);
  }

  _launchAtLoginEnabled() {
    try {
      return Gio.Subprocess.new(
        [
          'systemctl',
          '--user',
          'is-enabled',
          '--quiet',
          'keymap-overlay.service',
        ],
        Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE
      ).wait_check(null);
    } catch (_error) {
      return false;
    }
  }

  _setLaunchAtLogin(enabled) {
    const process = Gio.Subprocess.new(
      [
        'systemctl',
        '--user',
        enabled ? 'enable' : 'disable',
        'keymap-overlay.service',
      ],
      Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_PIPE
    );
    process.wait_check_async(null, (source, result) => {
      try {
        source.wait_check_finish(result);
      } catch (error) {
        console.error(
          `Keymap Overlay: cannot change launch at login: ${error.message}`
        );
        this._launchAtLoginItem?.setToggleState(!enabled);
      }
    });
  }

  _syncChoice(items, selected) {
    for (const [value, item] of items ?? [])
      item.setOrnament(
        value === selected ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE
      );
  }

  _updatePreferences(change) {
    const previous = { ...this._preferences };
    Object.assign(this._preferences, change);
    try {
      this._savePreferences();
    } catch (error) {
      this._preferences = previous;
      console.error(
        `Keymap Overlay: cannot save preferences: ${error.message}`
      );
    }
    this._syncIndicator();
    this._refreshVisibleOverlay();
  }

  _refreshVisibleOverlay() {
    if (this._lastVisible && this._lastModelJson) {
      this._generation--;
      this._applyState(this._generation + 1, true, this._lastModelJson);
    } else {
      this._hide();
    }
  }

  _syncColorScheme() {
    if (this._overlay)
      this._overlay.set_style_class_name(this._overlayStyleClass());
  }

  _overlayStyleClass() {
    const dark =
      this._interfaceSettings?.get_string('color-scheme') === 'prefer-dark';
    return dark
      ? 'popup-menu-content keymap-overlay keymap-overlay-dark'
      : 'keymap-overlay keymap-overlay-light';
  }

  _destroyOverlay() {
    if (!this._overlay) return;
    Main.layoutManager.removeChrome(this._overlay);
    this._overlay.destroy();
    this._overlay = null;
  }
}
