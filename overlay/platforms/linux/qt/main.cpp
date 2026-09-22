#include <QAccessible>
#include <QActionGroup>
#include <QApplication>
#include <QByteArray>
#include <QColor>
#include <QCursor>
#include <QDBusConnection>
#include <QDBusInterface>
#include <QDBusMessage>
#include <QDBusServiceWatcher>
#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QFont>
#include <QGuiApplication>
#include <QIcon>
#include <QJsonDocument>
#include <QJsonObject>
#include <QList>
#include <QMenu>
#include <QObject>
#include <QPalette>
#include <QProcess>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QQuickWindow>
#include <QSaveFile>
#include <QScreen>
#include <QStandardPaths>
#include <QString>
#include <QStringList>
#include <QStyle>
#include <QSystemTrayIcon>
#include <QTimer>
#include <QUrl>
#include <QVariant>
#include <QtGlobal>

#include <functional>
#include <initializer_list>
#include <memory>
#include <stdexcept>
#include <string>
#include <utility>

namespace {

constexpr auto BusName = "com.sunaemon.KeymapOverlay";
constexpr auto ObjectPath = "/com/sunaemon/KeymapOverlay";
constexpr auto RendererInterface = "com.sunaemon.KeymapOverlay.Renderer1";

struct Preferences {
  QString position = QStringLiteral("center");
  int opacity_percent = 100;
  int scale_percent = 100;
};

QString preferences_path() {
  const auto override = qEnvironmentVariable("KEYMAP_OVERLAY_PREFERENCES_FILE");
  if (!override.isEmpty())
    return override;
  return QStandardPaths::writableLocation(QStandardPaths::ConfigLocation) +
         QStringLiteral("/keymap-overlay/preferences.json");
}

Preferences load_preferences() {
  Preferences preferences;
  QFile file(preferences_path());
  if (!file.exists())
    return preferences;
  if (!file.open(QIODevice::ReadOnly))
    throw std::runtime_error("Failed to read overlay preferences");
  QJsonParseError error;
  const auto document = QJsonDocument::fromJson(file.readAll(), &error);
  if (error.error != QJsonParseError::NoError || !document.isObject())
    throw std::runtime_error("Failed to parse overlay preferences");
  const auto object = document.object();
  preferences.position = object.value(QStringLiteral("position"))
                             .toString(QStringLiteral("center"));
  preferences.opacity_percent =
      object.value(QStringLiteral("opacity_percent")).toInt(100);
  preferences.scale_percent =
      object.value(QStringLiteral("scale_percent")).toInt(100);
  if (!QStringList{QStringLiteral("top"), QStringLiteral("center"),
                   QStringLiteral("bottom")}
           .contains(preferences.position) ||
      !QList<int>{50, 75, 90, 100}.contains(preferences.opacity_percent) ||
      !QList<int>{75, 100, 125, 150}.contains(preferences.scale_percent))
    throw std::runtime_error(
        "Overlay preferences contain an unsupported value");
  return preferences;
}

void save_preferences(const Preferences &preferences) {
  const QFileInfo file_info(preferences_path());
  if (!QDir().mkpath(file_info.absolutePath()))
    throw std::runtime_error(
        "Failed to create the overlay preferences directory");
  QSaveFile file(file_info.absoluteFilePath());
  if (!file.open(QIODevice::WriteOnly))
    throw std::runtime_error("Failed to write overlay preferences");
  QJsonObject object{
      {QStringLiteral("position"), preferences.position},
      {QStringLiteral("opacity_percent"), preferences.opacity_percent},
      {QStringLiteral("scale_percent"), preferences.scale_percent}};
  if (file.write(QJsonDocument(object).toJson()) < 0 || !file.commit())
    throw std::runtime_error("Failed to save overlay preferences");
}

bool is_gnome_desktop() {
  const auto desktop = qEnvironmentVariable("XDG_CURRENT_DESKTOP");
  for (const auto &part : desktop.split(':')) {
    if (part.compare(QStringLiteral("gnome"), Qt::CaseInsensitive) == 0) {
      return true;
    }
  }
  return false;
}

constexpr auto LayerShellImportMarker = "// LAYER_SHELL_IMPORT";
constexpr auto LayerShellPropertiesMarker = "    // LAYER_SHELL_PROPERTIES";

constexpr auto OverlayQml = R"QML(
import QtQuick
import QtQuick.Window
// LAYER_SHELL_IMPORT

Window {
    id: root
    property var overlayModel: ({ keys: [], encoders: [] })
    property real contentScale: 1.0
    property string overlayPosition: "center"
    SystemPalette { id: systemPalette; colorGroup: SystemPalette.Active }

    visible: false
    color: "transparent"
    flags: Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
           | Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus
    width: (overlayModel.width || 1) * contentScale
    height: (overlayModel.height || 1) * contentScale
    contentItem.transform: Scale { xScale: root.contentScale; yScale: root.contentScale }

    // LAYER_SHELL_PROPERTIES

    Rectangle {
        id: panel
        objectName: "keymapOverlayPanel"
        width: root.overlayModel.width || 1
        height: root.overlayModel.height || 1
        radius: 22
        color: Qt.rgba(systemPalette.window.r, systemPalette.window.g, systemPalette.window.b, 0.90)
        border.width: 1
        border.color: systemPalette.mid
        Accessible.role: Accessible.Grouping
        Accessible.name: "Keymap Overlay L" + (root.overlayModel.layer ?? "")
        Accessible.focusable: false
    }

    Text {
        x: 20
        y: 20
        text: "L" + (root.overlayModel.layer ?? "")
        color: systemPalette.windowText
        font.pixelSize: root.overlayModel.header_font_size || 14
        Accessible.role: Accessible.StaticText
        Accessible.name: text
        Accessible.focusable: false
    }

    Repeater {
        model: root.overlayModel.keys || []
        delegate: Rectangle {
            required property var modelData
            x: modelData.x
            y: modelData.y
            width: modelData.width
            height: modelData.height
            radius: 11
            color: modelData.held ? systemPalette.highlight : systemPalette.button
            border.width: 1
            border.color: systemPalette.mid

            Text {
                anchors.centerIn: parent
                width: parent.width - 8
                text: modelData.label.join("\n")
                color: modelData.held ? systemPalette.highlightedText : systemPalette.buttonText
                font.pixelSize: root.overlayModel.key_font_size || 10
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                wrapMode: Text.Wrap
                Accessible.role: Accessible.StaticText
                Accessible.name: text
                Accessible.focusable: false
            }
        }
    }

    Repeater {
        model: root.overlayModel.encoders || []
        delegate: Item {
            required property var modelData
            x: modelData.x
            y: modelData.y
            width: modelData.size
            height: modelData.size

            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: modelData.held ? systemPalette.highlight : systemPalette.button
                border.width: 1
                border.color: systemPalette.mid
            }

            Text {
                anchors.centerIn: parent
                text: modelData.press ? "P " + modelData.press : ""
                color: modelData.held ? systemPalette.highlightedText : systemPalette.buttonText
                font.pixelSize: root.overlayModel.encoder_font_size || 10
                Accessible.role: Accessible.StaticText
                Accessible.name: text
                Accessible.focusable: false
            }

            Text {
                anchors.right: parent.horizontalCenter
                anchors.rightMargin: 3
                anchors.bottom: parent.top
                width: Math.max(0, parent.width * 0.75 - 3)
                clip: true
                text: modelData.counter_clockwise.length
                      ? "← " + modelData.counter_clockwise.join(" ") : ""
                color: systemPalette.windowText
                font.pixelSize: root.overlayModel.encoder_font_size || 10
                horizontalAlignment: Text.AlignRight
                Accessible.role: Accessible.StaticText
                Accessible.name: text
                Accessible.focusable: false
            }

            Text {
                anchors.left: parent.horizontalCenter
                anchors.leftMargin: 3
                anchors.bottom: parent.top
                width: Math.max(0, parent.width * 0.75 - 3)
                clip: true
                text: modelData.clockwise.length
                      ? modelData.clockwise.join(" ") + " →" : ""
                color: systemPalette.windowText
                font.pixelSize: root.overlayModel.encoder_font_size || 10
                Accessible.role: Accessible.StaticText
                Accessible.name: text
                Accessible.focusable: false
            }
        }
    }
}
)QML";

QByteArray overlay_qml() {
  auto source = QByteArray(OverlayQml);
  if (QGuiApplication::platformName().startsWith(QStringLiteral("wayland"))) {
    source.replace(LayerShellImportMarker,
                   "import org.kde.layershell 1.0 as LayerShell");
    source.replace(
        LayerShellPropertiesMarker,
        "    LayerShell.Window.anchors: root.overlayPosition === \"top\" ? "
        "LayerShell.Window.AnchorTop : (root.overlayPosition === \"bottom\" ? "
        "LayerShell.Window.AnchorBottom : LayerShell.Window.AnchorNone)\n"
        "    LayerShell.Window.exclusionZone: -1\n"
        "    LayerShell.Window.keyboardInteractivity: "
        "LayerShell.Window.KeyboardInteractivityNone\n"
        "    LayerShell.Window.layer: LayerShell.Window.LayerOverlay\n"
        "    LayerShell.Window.scope: \"keymap-overlay\"\n"
        "    LayerShell.Window.wantsToBeOnActiveScreen: true");
  } else {
    source.replace(LayerShellImportMarker, "");
    source.replace(LayerShellPropertiesMarker, "");
  }
  return source;
}

std::runtime_error qml_error(const QQmlComponent &component) {
  QStringList errors;
  for (const auto &error : component.errors()) {
    errors.append(error.toString());
  }
  return std::runtime_error(errors.join('\n').toStdString());
}

void apply_state(QQuickWindow &window, bool visible, const QString &model_json,
                 const Preferences &preferences) {
  if (!visible) {
    window.hide();
    return;
  }
  QJsonParseError parse_error;
  const auto document =
      QJsonDocument::fromJson(model_json.toUtf8(), &parse_error);
  if (parse_error.error != QJsonParseError::NoError || !document.isObject()) {
    throw std::runtime_error("Failed to parse an overlay model event: " +
                             parse_error.errorString().toStdString());
  }
  const auto model = document.object();
  const auto model_width = model.value(QStringLiteral("width")).toInt();
  const auto model_height = model.value(QStringLiteral("height")).toInt();
  const auto scale = preferences.scale_percent / 100.0;
  const auto width = qRound(model_width * scale);
  const auto height = qRound(model_height * scale);
  if (model.value(QStringLiteral("version")).toInt() != 2 || model_width <= 0 ||
      model_height <= 0) {
    throw std::runtime_error("The overlay model event is invalid");
  }
  window.setProperty("overlayModel", model.toVariantMap());
  window.setProperty("contentScale", scale);
  window.setProperty("overlayPosition", preferences.position);
  window.setOpacity(preferences.opacity_percent / 100.0);
  window.resize(width, height);
  if (QGuiApplication::platformName() == QStringLiteral("xcb")) {
    auto *screen = QGuiApplication::screenAt(QCursor::pos());
    if (!screen) {
      screen = QGuiApplication::primaryScreen();
    }
    if (screen) {
      const auto available = screen->availableGeometry();
      const auto x = available.x() + (available.width() - width) / 2;
      auto y = available.y() + (available.height() - height) / 2;
      if (preferences.position == QStringLiteral("top"))
        y = available.y();
      if (preferences.position == QStringLiteral("bottom"))
        y = available.bottom() - height + 1;
      window.setPosition(x, y);
    }
  }
  window.show();
}

void capture_golden_render(QQuickWindow &window) {
  const auto output = qEnvironmentVariable("KEYMAP_OVERLAY_GOLDEN_OUTPUT");
  if (output.isEmpty()) {
    return;
  }
  QTimer::singleShot(100, &window, [&window, output]() {
    if (!window.grabWindow().save(output)) {
      qCritical() << "Failed to save the golden render to" << output;
      QGuiApplication::exit(1);
    }
  });
}

void configure_golden_rendering(QGuiApplication &application) {
  if (!qEnvironmentVariableIsSet("KEYMAP_OVERLAY_GOLDEN_OUTPUT")) {
    return;
  }
  application.setFont(QFont(QStringLiteral("Noto Sans")));
  QPalette palette;
  palette.setColor(QPalette::Window, QColor(QStringLiteral("#f6f5f4")));
  palette.setColor(QPalette::WindowText, QColor(QStringLiteral("#2e3436")));
  palette.setColor(QPalette::Button, QColor(QStringLiteral("#deddda")));
  palette.setColor(QPalette::ButtonText, QColor(QStringLiteral("#2e3436")));
  palette.setColor(QPalette::Mid, QColor(QStringLiteral("#9a9996")));
  palette.setColor(QPalette::Highlight, QColor(QStringLiteral("#3584e4")));
  palette.setColor(QPalette::HighlightedText, Qt::white);
  application.setPalette(palette);
}

class RendererClient final : public QObject {
  Q_OBJECT

public:
  explicit RendererClient(QQuickWindow &window, Preferences preferences)
      : QObject(&window), window_(window), preferences_(std::move(preferences)),
        connection_(QDBusConnection::sessionBus()),
        owner_watcher_(QString::fromLatin1(BusName), connection_,
                       QDBusServiceWatcher::WatchForOwnerChange, this) {
    if (!connection_.isConnected()) {
      throw std::runtime_error("Failed to connect to the user D-Bus session");
    }
    QObject::connect(&owner_watcher_, &QDBusServiceWatcher::serviceOwnerChanged,
                     this, &RendererClient::service_owner_changed);
    if (!connection_.connect(QString::fromLatin1(BusName),
                             QString::fromLatin1(ObjectPath),
                             QString::fromLatin1(RendererInterface),
                             QStringLiteral("StateChanged"), this,
                             SLOT(state_changed(qulonglong, bool, QString)))) {
      throw std::runtime_error("Failed to subscribe to renderer state");
    }
    refresh_state();
  }

  const Preferences &preferences() const { return preferences_; }

  void set_preferences(const Preferences &preferences) {
    save_preferences(preferences);
    preferences_ = preferences;
    apply_state(window_, visible_, model_json_, preferences_);
  }

private slots:
  void state_changed(qulonglong generation, bool visible,
                     const QString &model_json) {
    try {
      apply_update(generation, visible, model_json);
    } catch (const std::exception &error) {
      fail(error);
    }
  }

private:
  void service_owner_changed(const QString &, const QString &,
                             const QString &new_owner) {
    generation_ = 0;
    if (new_owner.isEmpty()) {
      visible_ = false;
      model_json_.clear();
      window_.hide();
      return;
    }
    try {
      refresh_state();
    } catch (const std::exception &error) {
      fail(error);
    }
  }

  void refresh_state() {
    QDBusInterface renderer(
        QString::fromLatin1(BusName), QString::fromLatin1(ObjectPath),
        QString::fromLatin1(RendererInterface), connection_);
    if (!renderer.isValid()) {
      throw std::runtime_error("The renderer D-Bus service is unavailable: " +
                               renderer.lastError().message().toStdString());
    }
    const auto reply = renderer.call(QStringLiteral("GetState"));
    if (reply.type() == QDBusMessage::ErrorMessage) {
      throw std::runtime_error("Failed to read renderer state: " +
                               reply.errorMessage().toStdString());
    }
    const auto arguments = reply.arguments();
    if (arguments.size() != 3) {
      throw std::runtime_error("The renderer D-Bus state has an invalid shape");
    }
    bool generation_ok = false;
    const auto generation = arguments.at(0).toULongLong(&generation_ok);
    if (!generation_ok || !arguments.at(1).canConvert<bool>() ||
        !arguments.at(2).canConvert<QString>()) {
      throw std::runtime_error("The renderer D-Bus state has invalid types");
    }
    apply_update(generation, arguments.at(1).toBool(),
                 arguments.at(2).toString());
  }

  void apply_update(qulonglong generation, bool visible,
                    const QString &model_json) {
    if (generation <= generation_) {
      return;
    }
    generation_ = generation;
    visible_ = visible;
    model_json_ = model_json;
    apply_state(window_, visible_, model_json_, preferences_);
    if (visible && !captured_golden_render_) {
      captured_golden_render_ = true;
      capture_golden_render(window_);
    }
  }

  static void fail(const std::exception &error) {
    qCritical() << error.what();
    QGuiApplication::exit(1);
  }

  QQuickWindow &window_;
  Preferences preferences_;
  QDBusConnection connection_;
  QDBusServiceWatcher owner_watcher_;
  qulonglong generation_ = 0;
  bool visible_ = false;
  QString model_json_;
  bool captured_golden_render_ = false;
};

class TrayMenu final : public QObject {
public:
  explicit TrayMenu(RendererClient &renderer, QObject *parent = nullptr)
      : QObject(parent), renderer_(renderer), menu_(new QMenu),
        tray_(new QSystemTrayIcon(
            QIcon::fromTheme(QStringLiteral("input-keyboard")), this)) {
    if (tray_->icon().isNull())
      tray_->setIcon(
          QApplication::style()->standardIcon(QStyle::SP_ComputerIcon));
    rebuild();
    tray_->setToolTip(QStringLiteral("Keymap Overlay"));
    tray_->setContextMenu(menu_);
    tray_->show();
  }

  ~TrayMenu() override {
    tray_->setContextMenu(nullptr);
    delete menu_;
  }

private:
  void rebuild() {
    menu_->clear();
    auto preferences = renderer_.preferences();
    auto *launch_at_login = menu_->addAction(QStringLiteral("Launch at Login"));
    launch_at_login->setCheckable(true);
    launch_at_login->setChecked(is_launch_at_login_enabled());
    connect(launch_at_login, &QAction::triggered, this, [this](bool enabled) {
      const auto action =
          enabled ? QStringLiteral("enable") : QStringLiteral("disable");
      const auto result =
          QProcess::execute(QStringLiteral("systemctl"),
                            {QStringLiteral("--user"), action,
                             QStringLiteral("keymap-overlay.service"),
                             QStringLiteral("keymap-overlay-qt.service")});
      if (result != 0)
        qCritical() << "Failed to change the launch-at-login setting";
      rebuild();
    });

    auto *position = menu_->addMenu(QStringLiteral("Position"));
    auto *position_group = new QActionGroup(position);
    for (const auto &choice : {QStringLiteral("top"), QStringLiteral("center"),
                               QStringLiteral("bottom")}) {
      auto *action = position->addAction(choice[0].toUpper() + choice.mid(1));
      action->setCheckable(true);
      action->setChecked(preferences.position == choice);
      position_group->addAction(action);
      connect(action, &QAction::triggered, this, [this, choice] {
        auto next = renderer_.preferences();
        next.position = choice;
        update(next);
      });
    }

    add_percentage_menu(
        QStringLiteral("Opacity"), {50, 75, 90, 100},
        preferences.opacity_percent,
        [this](Preferences &next, int value) { next.opacity_percent = value; });
    add_percentage_menu(
        QStringLiteral("Scale"), {75, 100, 125, 150}, preferences.scale_percent,
        [this](Preferences &next, int value) { next.scale_percent = value; });
    menu_->addSeparator();
    auto *reload = menu_->addAction(QStringLiteral("Reload Keyboards"));
    connect(reload, &QAction::triggered, this, [] {
      const auto message = QDBusMessage::createMethodCall(
          QString::fromUtf8(BusName), QString::fromUtf8(ObjectPath),
          QString::fromUtf8(RendererInterface),
          QStringLiteral("ReloadKeyboards"));
      QDBusConnection::sessionBus().asyncCall(message);
    });
    auto *version =
        menu_->addAction(QStringLiteral("Keymap Overlay %1")
                             .arg(QStringLiteral(KEYMAP_OVERLAY_VERSION)));
    version->setEnabled(false);
    auto *quit = menu_->addAction(QStringLiteral("Quit"));
    connect(quit, &QAction::triggered, this, [] {
      QProcess::startDetached(QStringLiteral("systemctl"),
                              {QStringLiteral("--user"), QStringLiteral("stop"),
                               QStringLiteral("keymap-overlay.service"),
                               QStringLiteral("keymap-overlay-qt.service")});
      QCoreApplication::quit();
    });
  }

  void
  add_percentage_menu(const QString &title, std::initializer_list<int> values,
                      int selected,
                      const std::function<void(Preferences &, int)> &setter) {
    auto *submenu = menu_->addMenu(title);
    auto *group = new QActionGroup(submenu);
    for (const auto value : values) {
      auto *action = submenu->addAction(QStringLiteral("%1%").arg(value));
      action->setCheckable(true);
      action->setChecked(value == selected);
      group->addAction(action);
      connect(action, &QAction::triggered, this, [this, setter, value] {
        auto next = renderer_.preferences();
        setter(next, value);
        update(next);
      });
    }
  }

  void update(const Preferences &preferences) {
    try {
      renderer_.set_preferences(preferences);
      rebuild();
    } catch (const std::exception &error) {
      qCritical() << error.what();
    }
  }

  static bool is_launch_at_login_enabled() {
    return QProcess::execute(QStringLiteral("systemctl"),
                             {QStringLiteral("--user"),
                              QStringLiteral("is-enabled"),
                              QStringLiteral("--quiet"),
                              QStringLiteral("keymap-overlay.service")}) == 0;
  }

  RendererClient &renderer_;
  QMenu *menu_;
  QSystemTrayIcon *tray_;
};

} // namespace

int main(int argc, char *argv[]) {
  if (is_gnome_desktop() &&
      !qEnvironmentVariableIsSet("KEYMAP_OVERLAY_FORCE_QT")) {
    return 0;
  }

  QApplication application(argc, argv);
  QGuiApplication::setApplicationName(QStringLiteral("keymap-overlay-qt"));
  QAccessible::setActive(true);
  configure_golden_rendering(application);

  try {
    QQmlEngine engine;
    QQmlComponent component(&engine);
    component.setData(overlay_qml(),
                      QUrl(QStringLiteral("qrc:/keymap-overlay.qml")));
    if (component.isError()) {
      throw qml_error(component);
    }
    std::unique_ptr<QObject> root(component.create());
    if (!root) {
      throw qml_error(component);
    }
    auto *window = qobject_cast<QQuickWindow *>(root.get());
    if (!window) {
      throw std::runtime_error("The Qt overlay root is not a window");
    }

    RendererClient renderer(*window, load_preferences());
    std::unique_ptr<TrayMenu> tray;
    if (QGuiApplication::platformName() != QStringLiteral("offscreen"))
      tray = std::make_unique<TrayMenu>(renderer);

    return application.exec();
  } catch (const std::exception &error) {
    qCritical() << error.what();
    return 1;
  }
}

#include "main.moc"
