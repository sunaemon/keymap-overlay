using System.IO;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Shapes;
using IOPath = System.IO.Path;

namespace KeymapOverlay;

/// <summary>Renders layer models in a non-activating native Windows overlay.</summary>
internal sealed class OverlayWindow : Window
{
    private const uint MonitorDefaultToNearest = 2;
    private const uint TransitionKindMask = 0xff000000;
    private const uint TransitionShow = 2 << 24;
    private static readonly Brush KeyFill = Brush("#F6DCE0E7");
    private static readonly Brush HeldFill = Brush("#FFFFDDDD");
    private static readonly Brush KeyBorder = Brush("#1F20242C");
    private static readonly Brush TextFill = Brush("#FF20242C");

    private readonly Dictionary<(byte Keyboard, byte Layer), OverlayModel> models;
    private readonly NativeMethods.WakeCallback wakeCallback;
    private nint handle;

    internal OverlayWindow(string assetsDirectory)
    {
        models = LoadModels(assetsDirectory);
        wakeCallback = WakeUi;
        WindowStyle = WindowStyle.None;
        AllowsTransparency = true;
        Background = Brushes.Transparent;
        ShowInTaskbar = false;
        ShowActivated = false;
        Topmost = true;
        ResizeMode = ResizeMode.NoResize;
        IsHitTestVisible = false;
        Width = 1;
        Height = 1;
        Content = new Canvas { Background = Brushes.Transparent };
        SourceInitialized += ConfigureNativeWindow;
    }

    internal void StartListener()
    {
        var status = NativeMethods.Start(wakeCallback);
        if (status != 0)
        {
            throw new InvalidOperationException($"Rust HID listener failed to start ({status}).");
        }
    }

    private static Dictionary<(byte, byte), OverlayModel> LoadModels(string directory)
    {
        var result = new Dictionary<(byte, byte), OverlayModel>();
        string[] paths;
        try
        {
            paths = Directory.EnumerateFiles(directory, "*_L*.json").ToArray();
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException)
        {
            return result;
        }

        foreach (var path in paths)
        {
            try
            {
                var model = JsonSerializer.Deserialize<OverlayModel>(File.ReadAllText(path));
                var stem = IOPath.GetFileNameWithoutExtension(path);
                var parts = stem.Split("_L", StringSplitOptions.None);
                if (model is not null && model.Version == 1 && parts.Length == 2 &&
                    byte.TryParse(parts[0], out var keyboard) && byte.TryParse(parts[1], out var layer) &&
                    model.Layer == layer)
                {
                    result[(keyboard, layer)] = model;
                }
            }
            catch (Exception error) when (error is JsonException or IOException or UnauthorizedAccessException)
            {
                // A bad model is unavailable just like a missing one; other
                // installed keyboards remain usable.
            }
        }
        return result;
    }

    private void ConfigureNativeWindow(object? sender, EventArgs args)
    {
        handle = new WindowInteropHelper(this).Handle;
        var styles = NativeMethods.GetWindowLongPtr(handle, NativeMethods.GwlExStyle).ToInt64();
        styles |= NativeMethods.WsExTransparent | NativeMethods.WsExToolWindow | NativeMethods.WsExNoActivate;
        NativeMethods.SetWindowLongPtr(handle, NativeMethods.GwlExStyle, new nint(styles));
    }

    private void WakeUi()
    {
        if (!Dispatcher.HasShutdownStarted)
        {
            _ = Dispatcher.BeginInvoke(DrainTransition);
        }
    }

    private void DrainTransition()
    {
        var transition = NativeMethods.TakeTransition();
        if (transition == 1)
        {
            HideOverlay();
        }
        else if ((transition & TransitionKindMask) == TransitionShow)
        {
            var keyboard = (byte)((transition >> 8) & 0xff);
            var layer = (byte)(transition & 0xff);
            ShowOverlay(keyboard, layer);
        }
    }

    private void ShowOverlay(byte keyboard, byte layer)
    {
        if (!models.TryGetValue((keyboard, layer), out var model))
        {
            HideOverlay();
            return;
        }

        Content = BuildCanvas(model);
        Width = model.Width;
        Height = model.Height;
        PositionOnCursorMonitor(model.Width, model.Height);
    }

    private void HideOverlay()
    {
        Content = new Canvas { Background = Brushes.Transparent };
        Width = 1;
        Height = 1;
        NativeMethods.SetWindowPos(handle, NativeMethods.HwndTopmost, 0, 0, 1, 1, NativeMethods.SwpNoActivate);
    }

    private void PositionOnCursorMonitor(double width, double height)
    {
        if (!NativeMethods.GetCursorPos(out var cursor))
        {
            return;
        }
        var monitor = NativeMethods.MonitorFromPoint(cursor, MonitorDefaultToNearest);
        var info = new NativeMethods.MonitorInfo { Size = (uint)Marshal.SizeOf<NativeMethods.MonitorInfo>() };
        if (!NativeMethods.GetMonitorInfo(monitor, ref info))
        {
            return;
        }

        double dpiScaleX;
        double dpiScaleY;
        if (NativeMethods.GetDpiForMonitor(
                monitor,
                NativeMethods.MonitorDpiType.Effective,
                out var dpiX,
                out var dpiY) == 0)
        {
            dpiScaleX = dpiX / 96.0;
            dpiScaleY = dpiY / 96.0;
        }
        else
        {
            var dpi = VisualTreeHelper.GetDpi(this);
            dpiScaleX = dpi.DpiScaleX;
            dpiScaleY = dpi.DpiScaleY;
        }

        var pixelWidth = (int)Math.Round(width * dpiScaleX);
        var pixelHeight = (int)Math.Round(height * dpiScaleY);
        var x = info.Work.Left + (info.Work.Right - info.Work.Left - pixelWidth) / 2;
        var y = info.Work.Top + (info.Work.Bottom - info.Work.Top - pixelHeight) / 2;
        NativeMethods.SetWindowPos(
            handle,
            NativeMethods.HwndTopmost,
            x,
            y,
            pixelWidth,
            pixelHeight,
            NativeMethods.SwpNoActivate);
    }

    private static Canvas BuildCanvas(OverlayModel model)
    {
        var canvas = new Canvas { Width = model.Width, Height = model.Height, Background = Brushes.Transparent };
        AddText(canvas, $"L{model.Layer}", 20, 14, model.Width - 40, 30, model.HeaderFontSize, TextAlignment.Left);
        foreach (var key in model.Keys)
        {
            var surface = new Border
            {
                Width = key.Width,
                Height = key.Height,
                CornerRadius = new CornerRadius(11),
                Background = key.Held ? HeldFill : KeyFill,
                BorderBrush = KeyBorder,
                BorderThickness = new Thickness(1),
                Child = Label(string.Join('\n', key.Label), model.KeyFontSize, TextAlignment.Center),
            };
            Canvas.SetLeft(surface, key.X);
            Canvas.SetTop(surface, key.Y);
            canvas.Children.Add(surface);
        }
        foreach (var encoder in model.Encoders)
        {
            AddEncoder(canvas, encoder, model.EncoderFontSize);
        }
        return canvas;
    }

    private static void AddEncoder(Canvas canvas, DisplayEncoder encoder, double fontSize)
    {
        var knob = new Ellipse
        {
            Width = encoder.Size,
            Height = encoder.Size,
            Fill = encoder.Held ? HeldFill : KeyFill,
            Stroke = KeyBorder,
            StrokeThickness = 1,
        };
        Canvas.SetLeft(knob, encoder.X);
        Canvas.SetTop(knob, encoder.Y);
        canvas.Children.Add(knob);
        AddText(canvas, $"↶ {string.Join(' ', encoder.CounterClockwise)}", encoder.X - encoder.Size, encoder.Y - 26, encoder.Size * 1.5, 24, fontSize, TextAlignment.Right);
        AddText(canvas, $"{string.Join(' ', encoder.Clockwise)} ↷", encoder.X + encoder.Size / 2, encoder.Y - 26, encoder.Size * 1.5, 24, fontSize, TextAlignment.Left);
        AddText(canvas, string.IsNullOrEmpty(encoder.Press) ? "" : $"P {encoder.Press}", encoder.X, encoder.Y, encoder.Size, encoder.Size, fontSize, TextAlignment.Center);
    }

    private static void AddText(Canvas canvas, string text, double x, double y, double width, double height, double fontSize, TextAlignment alignment)
    {
        var label = Label(text, fontSize, alignment);
        label.Width = width;
        label.Height = height;
        Canvas.SetLeft(label, x);
        Canvas.SetTop(label, y);
        canvas.Children.Add(label);
    }

    private static TextBlock Label(string text, double fontSize, TextAlignment alignment) => new()
    {
        Text = text,
        Foreground = TextFill,
        FontFamily = new FontFamily("Segoe UI"),
        FontSize = fontSize,
        TextAlignment = alignment,
        TextWrapping = TextWrapping.Wrap,
        VerticalAlignment = VerticalAlignment.Center,
        HorizontalAlignment = HorizontalAlignment.Stretch,
    };

    private static Brush Brush(string color)
    {
        var brush = new SolidColorBrush((Color)ColorConverter.ConvertFromString(color));
        brush.Freeze();
        return brush;
    }
}
