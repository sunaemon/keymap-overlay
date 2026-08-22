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
            var options = ParseArguments(e.Args);
            var assetsDirectory = options.AssetsDirectory
                // Local rather than roaming: the models are generated and describe
                // one machine.
                ?? Path.Combine(
                    Environment.GetFolderPath(
                        Environment.SpecialFolder.LocalApplicationData,
                        Environment.SpecialFolderOption.Create),
                    "keymap-overlay");

            if (options.KeyboardConfigDirectory is not null && NativeMethods.Prepare() != 0)
            {
                throw new InvalidOperationException("Rust model refresh failed to start.");
            }
            var window = new OverlayWindow(assetsDirectory);
            MainWindow = window;
            window.Show();
            window.StartListener(options.KeyboardId, options.Layer);
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

    /// <summary>Parses the model directories and optional simulated layer.</summary>
    private static (
        string? AssetsDirectory,
        string? KeyboardConfigDirectory,
        byte? KeyboardId,
        byte? Layer) ParseArguments(string[] arguments)
    {
        string? directory = null;
        string? keyboardConfigDirectory = null;
        byte? keyboardId = null;
        byte? layer = null;
        for (var index = 0; index < arguments.Length; index++)
        {
            var option = arguments[index];
            index++;
            if (index == arguments.Length || arguments[index].StartsWith("-", StringComparison.Ordinal))
            {
                throw new ArgumentException($"{option} requires a value.");
            }

            if (option == "--asset-dir")
            {
                if (directory is not null)
                {
                    throw new ArgumentException("--asset-dir may only be specified once.");
                }
                directory = arguments[index];
                continue;
            }

            if (option == "--keyboard-config-dir")
            {
                if (keyboardConfigDirectory is not null)
                {
                    throw new ArgumentException("--keyboard-config-dir may only be specified once.");
                }
                keyboardConfigDirectory = arguments[index];
                continue;
            }

            if (option != "--simulate")
            {
                throw new ArgumentException(
                    $"Unknown argument '{option}'. Expected --asset-dir PATH, --keyboard-config-dir PATH, or --simulate KEYBOARD_ID:LAYER.");
            }
            if (keyboardId is not null)
            {
                throw new ArgumentException("--simulate may only be specified once.");
            }
            var parts = arguments[index].Split(':');
            if (parts.Length != 2 || !byte.TryParse(parts[0], out var parsedKeyboardId) ||
                !byte.TryParse(parts[1], out var parsedLayer))
            {
                throw new ArgumentException("--simulate requires KEYBOARD_ID:LAYER, each from 0 to 255.");
            }
            keyboardId = parsedKeyboardId;
            layer = parsedLayer;
        }
        return (directory, keyboardConfigDirectory, keyboardId, layer);
    }
}
