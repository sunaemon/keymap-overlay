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
            if (NativeMethods.Prepare() != 0)
            {
                throw new InvalidOperationException("Vial model loading failed.");
            }
            var window = new OverlayWindow();
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

    /// <summary>Parses the optional simulated layer.</summary>
    private static (byte? KeyboardId, byte? Layer) ParseArguments(string[] arguments)
    {
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

            if (option != "--simulate")
            {
                throw new ArgumentException(
                    $"Unknown argument '{option}'. Expected --simulate KEYBOARD_ID:LAYER.");
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
        return (keyboardId, layer);
    }
}
