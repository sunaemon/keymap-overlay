using System.IO;
using System.Windows;

namespace KeymapOverlay;

/// <summary>Owns WPF startup and shutdown for the overlay.</summary>
public partial class App : Application
{
    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        try
        {
            var assetsDirectory = AssetDirectory(e.Args)
                // Local rather than roaming: the models are generated and describe
                // one machine.
                ?? Path.Combine(
                    Environment.GetFolderPath(
                        Environment.SpecialFolder.LocalApplicationData,
                        Environment.SpecialFolderOption.Create),
                    "keymap-overlay");

            var window = new OverlayWindow(assetsDirectory);
            MainWindow = window;
            window.Show();
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

    /// <summary>Returns the --asset-dir value, or null when it is absent.</summary>
    private static string? AssetDirectory(string[] arguments)
    {
        string? directory = null;
        for (var index = 0; index < arguments.Length; index++)
        {
            if (arguments[index] != "--asset-dir")
            {
                throw new ArgumentException(
                    $"Unknown argument '{arguments[index]}'. Expected --asset-dir PATH.");
            }

            if (directory is not null)
            {
                throw new ArgumentException("--asset-dir may only be specified once.");
            }

            index++;
            if (index == arguments.Length || arguments[index].StartsWith("-", StringComparison.Ordinal))
            {
                throw new ArgumentException("--asset-dir requires a path.");
            }

            directory = arguments[index];
        }
        return directory;
    }
}
