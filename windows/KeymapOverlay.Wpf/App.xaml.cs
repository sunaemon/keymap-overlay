using System.IO;
using System.Windows;

namespace KeymapOverlay;

/// <summary>Owns WPF startup and shutdown for the overlay.</summary>
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
        try
        {
            window.StartListener();
        }
        catch (Exception error)
        {
            MessageBox.Show(
                error.Message,
                "Keymap Overlay",
                MessageBoxButton.OK,
                MessageBoxImage.Error);
            Shutdown(1);
        }
    }
}
