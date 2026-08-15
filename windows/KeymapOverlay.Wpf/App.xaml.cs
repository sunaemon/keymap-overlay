using System.IO;
using System.Windows;

namespace KeymapOverlay;

public partial class App : Application
{
    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        var assetsDirectory = e.Args.Length > 0
            ? e.Args[0]
            : Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                ".config",
                "keymap-overlay");

        var window = new OverlayWindow(assetsDirectory);
        MainWindow = window;
        window.Show();
        window.StartListener();
    }
}
