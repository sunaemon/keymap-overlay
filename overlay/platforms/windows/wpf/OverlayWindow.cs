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
    private const int DeviceNodesChanged = 0x0007;
    private const int WindowMessageDeviceChange = 0x0219;
    private const double OverlayBorderThickness = 1;
    private const double HeaderHorizontalInset = 20;
    private const double EncoderLabelWidthRatio = 0.7;
    private const double EncoderLabelGap = 3;
    private const double EncoderLabelVerticalOffset = 30;
    private static readonly Brush KeyFill = Brush("#E0F1F4F8");
    private static readonly Brush HeldFill = Brush("#FFFFDDDD");
    private static readonly Brush KeyBorder = Brush("#6020242C");
    private static readonly Brush OverlayFill = Brush("#E8D8E0EA");
    private static readonly Brush OverlayBorder = Brush("#70606773");
    private static readonly Brush TextFill = Brush("#FF20242C");

    private readonly Dictionary<(byte Keyboard, byte Layer), OverlayModel> models;
    private readonly Dictionary<(byte Keyboard, string Layers), OverlayModel> composedModels = [];
    private readonly NativeMethods.WakeCallback wakeCallback;
    private readonly string? e2eStateFile = Environment.GetEnvironmentVariable("KEYMAP_OVERLAY_E2E_STATE_FILE");
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

    internal void StartListener(byte? keyboardId, byte? layer)
    {
        var status = keyboardId is byte simulatedKeyboardId && layer is byte simulatedLayer
            ? NativeMethods.StartSimulated(wakeCallback, simulatedKeyboardId, simulatedLayer)
            : NativeMethods.Start(wakeCallback);
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
            paths = Directory.EnumerateFiles(directory, "*.json").ToArray();
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException)
        {
            return result;
        }

        foreach (var path in paths)
        {
            var stem = IOPath.GetFileNameWithoutExtension(path);
            if (!byte.TryParse(stem, out var keyboard))
            {
                continue;
            }
            try
            {
                var keyboardModels = JsonSerializer.Deserialize<KeyboardModels>(File.ReadAllText(path));
                var layers = keyboardModels?.Layers;
                if (keyboardModels is null || keyboardModels.KeyboardId != keyboard || layers is null ||
                    !layers.TryGetValue(0, out var baseModel) || !IsValidModel(baseModel, 0) ||
                    layers.Any(pair => !IsValidModel(pair.Value, pair.Key)))
                {
                    continue;
                }
                foreach (var (layer, model) in layers)
                {
                    result[(keyboard, layer)] = model!;
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

    private static bool IsValidModel(OverlayModel? model, byte layer) =>
        model is not null &&
        (model.Version == 1 || model.Version == 2) &&
        model.Layer == layer &&
        IsPositiveFinite(model.Width) &&
        IsPositiveFinite(model.Height) &&
        IsNonNegativeFinite(HeaderWidth(model)) &&
        IsPositiveFinite(model.Width + 2 * OverlayBorderThickness) &&
        IsPositiveFinite(model.Height + 2 * OverlayBorderThickness) &&
        IsPositiveFinite(model.HeaderFontSize) &&
        IsPositiveFinite(model.KeyFontSize) &&
        IsPositiveFinite(model.EncoderFontSize) &&
        model.Keys is not null && model.Keys.All(IsValidKey) &&
        model.Encoders is not null && model.Encoders.All(IsValidEncoder);

    private static bool IsValidKey(DisplayKey? key) =>
        key is not null &&
        double.IsFinite(key.X) &&
        double.IsFinite(key.Y) &&
        IsNonNegativeFinite(key.Width) &&
        IsNonNegativeFinite(key.Height) &&
        double.IsFinite(key.X + key.Width) &&
        double.IsFinite(key.Y + key.Height) &&
        key.Label is not null;

    private static bool IsValidEncoder(DisplayEncoder? encoder)
    {
        if (encoder is null ||
            !double.IsFinite(encoder.X) ||
            !double.IsFinite(encoder.Y) ||
            !IsPositiveFinite(encoder.Size) ||
            encoder.CounterClockwise is null ||
            encoder.Clockwise is null ||
            encoder.Press is null)
        {
            return false;
        }

        var centerX = EncoderCenterX(encoder);
        var labelWidth = EncoderLabelWidth(encoder);
        return double.IsFinite(encoder.X + encoder.Size) &&
            double.IsFinite(encoder.Y + encoder.Size) &&
            double.IsFinite(centerX) &&
            IsPositiveFinite(labelWidth) &&
            double.IsFinite(centerX - labelWidth - EncoderLabelGap / 2) &&
            double.IsFinite(centerX + EncoderLabelGap / 2) &&
            double.IsFinite(encoder.Y - EncoderLabelVerticalOffset);
    }

    private static bool IsPositiveFinite(double value) => value > 0 && double.IsFinite(value);

    private static bool IsNonNegativeFinite(double value) => value >= 0 && double.IsFinite(value);

    private static double HeaderWidth(OverlayModel model) => model.Width - 2 * HeaderHorizontalInset;

    private static double EncoderCenterX(DisplayEncoder encoder) => encoder.X + encoder.Size / 2;

    private static double EncoderLabelWidth(DisplayEncoder encoder) => encoder.Size * EncoderLabelWidthRatio;

    private void ConfigureNativeWindow(object? sender, EventArgs args)
    {
        handle = new WindowInteropHelper(this).Handle;
        HwndSource.FromHwnd(handle)?.AddHook(ProcessWindowMessage);
        var styles = NativeMethods.GetWindowLongPtr(handle, NativeMethods.GwlExStyle).ToInt64();
        styles |= NativeMethods.WsExTransparent | NativeMethods.WsExToolWindow | NativeMethods.WsExNoActivate;
        NativeMethods.SetWindowLongPtr(handle, NativeMethods.GwlExStyle, new nint(styles));
    }

    private static nint ProcessWindowMessage(
        nint window,
        int message,
        nint parameter,
        nint data,
        ref bool handled)
    {
        if (message == WindowMessageDeviceChange && parameter == DeviceNodesChanged)
        {
            NativeMethods.DeviceArrived();
        }
        return 0;
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
            var layerCount = (byte)((transition >> 16) & 0xff);
            var layers = layerCount == 0
                ? [(byte)(transition & 0xff)]
                : Enumerable.Range(0, layerCount)
                    .Select(index => NativeMethods.TransitionLayer((byte)index))
                    .ToArray();
            ShowOverlay(keyboard, layers);
        }
    }

    private void ShowOverlay(byte keyboard, byte[] layers)
    {
        var key = (keyboard, string.Join(',', layers));
        if (!composedModels.TryGetValue(key, out var model))
        {
            model = ComposeModel(keyboard, layers);
            if (model is not null)
            {
                composedModels[key] = model;
            }
        }
        if (model is null)
        {
            HideOverlay();
            return;
        }

        var width = model.Width + 2 * OverlayBorderThickness;
        var height = model.Height + 2 * OverlayBorderThickness;
        Content = BuildCanvas(model);
        Width = width;
        Height = height;
        PositionOnCursorMonitor(width, height);
        var heldCount = model.Keys.Count(key => key.Held) + model.Encoders.Count(encoder => encoder.Held);
        RecordE2eState(
            $"show keyboard={keyboard} layers=[{string.Join(',', layers)}] size={width}x{height} " +
            $"keys={model.Keys.Count} encoders={model.Encoders.Count} held={heldCount}");
    }

    private OverlayModel? ComposeModel(byte keyboard, byte[] layers)
    {
        if (!models.TryGetValue((keyboard, 0), out var baseModel))
        {
            return null;
        }
        var model = CloneModel(baseModel);
        foreach (var layer in layers)
        {
            if (!models.TryGetValue((keyboard, layer), out var overlay) ||
                overlay.Keys.Count != model.Keys.Count ||
                overlay.Encoders.Count != model.Encoders.Count)
            {
                return null;
            }
            ApplyOverlay(model, overlay);
            model.Layer = layer;
        }
        foreach (var displayKey in model.Keys)
        {
            displayKey.Held = displayKey.MomentaryLayer is byte layer && layers.Contains(layer);
        }
        foreach (var encoder in model.Encoders)
        {
            encoder.Held = encoder.MomentaryLayer is byte layer && layers.Contains(layer);
        }
        model.Version = 2;
        return model;
    }

    private static void ApplyOverlay(OverlayModel model, OverlayModel overlay)
    {
        for (var index = 0; index < model.Keys.Count; index++)
        {
            if (!overlay.Keys[index].Transparent)
            {
                model.Keys[index] = CloneKey(overlay.Keys[index]);
            }
        }
        for (var index = 0; index < model.Encoders.Count; index++)
        {
            var target = model.Encoders[index];
            var source = overlay.Encoders[index];
            if (!source.CounterClockwiseTransparent)
            {
                target.CounterClockwise = [.. source.CounterClockwise];
            }
            if (!source.ClockwiseTransparent)
            {
                target.Clockwise = [.. source.Clockwise];
            }
            if (!source.PressTransparent)
            {
                target.Press = source.Press;
                target.MomentaryLayer = source.MomentaryLayer;
            }
        }
    }

    private static OverlayModel CloneModel(OverlayModel model) => new()
    {
        Version = model.Version,
        Layer = model.Layer,
        Width = model.Width,
        Height = model.Height,
        HeaderFontSize = model.HeaderFontSize,
        KeyFontSize = model.KeyFontSize,
        EncoderFontSize = model.EncoderFontSize,
        Keys = model.Keys.Select(CloneKey).ToList(),
        Encoders = model.Encoders.Select(CloneEncoder).ToList(),
    };

    private static DisplayKey CloneKey(DisplayKey key) => new()
    {
        X = key.X,
        Y = key.Y,
        Width = key.Width,
        Height = key.Height,
        Label = [.. key.Label],
        Held = key.Held,
        Transparent = key.Transparent,
        MomentaryLayer = key.MomentaryLayer,
    };

    private static DisplayEncoder CloneEncoder(DisplayEncoder encoder) => new()
    {
        X = encoder.X,
        Y = encoder.Y,
        Size = encoder.Size,
        CounterClockwise = [.. encoder.CounterClockwise],
        Clockwise = [.. encoder.Clockwise],
        Press = encoder.Press,
        Held = encoder.Held,
        CounterClockwiseTransparent = encoder.CounterClockwiseTransparent,
        ClockwiseTransparent = encoder.ClockwiseTransparent,
        PressTransparent = encoder.PressTransparent,
        MomentaryLayer = encoder.MomentaryLayer,
    };

    private void HideOverlay()
    {
        Content = new Canvas { Background = Brushes.Transparent };
        Width = 1;
        Height = 1;
        NativeMethods.SetWindowPos(handle, NativeMethods.HwndTopmost, 0, 0, 1, 1, NativeMethods.SwpNoActivate);
        RecordE2eState("hide size=1x1");
    }

    private void RecordE2eState(string state)
    {
        if (e2eStateFile is null)
        {
            return;
        }
        try
        {
            File.AppendAllText(e2eStateFile, state + Environment.NewLine);
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException)
        {
            Console.Error.WriteLine($"Failed to record Windows E2E state in {e2eStateFile}: {error}");
        }
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

    private static Border BuildCanvas(OverlayModel model)
    {
        var canvas = new Canvas { Width = model.Width, Height = model.Height, Background = Brushes.Transparent };
        AddText(canvas, $"L{model.Layer}", HeaderHorizontalInset, 14, HeaderWidth(model), 30, model.HeaderFontSize, TextAlignment.Left);
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
        return new Border
        {
            Width = model.Width + 2 * OverlayBorderThickness,
            Height = model.Height + 2 * OverlayBorderThickness,
            CornerRadius = new CornerRadius(16),
            Background = OverlayFill,
            BorderBrush = OverlayBorder,
            BorderThickness = new Thickness(OverlayBorderThickness),
            Child = canvas,
        };
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
        var centerX = EncoderCenterX(encoder);
        var labelWidth = EncoderLabelWidth(encoder);
        var labelTop = encoder.Y - EncoderLabelVerticalOffset;
        AddText(canvas, string.Join(' ', encoder.CounterClockwise), centerX - labelWidth - EncoderLabelGap / 2, labelTop, labelWidth, 26, fontSize, TextAlignment.Center);
        AddText(canvas, string.Join(' ', encoder.Clockwise), centerX + EncoderLabelGap / 2, labelTop, labelWidth, 26, fontSize, TextAlignment.Center);
        AddText(canvas, string.IsNullOrEmpty(encoder.Press) ? "" : $"P {encoder.Press}", encoder.X, encoder.Y, encoder.Size, encoder.Size, fontSize, TextAlignment.Center);
    }

    private static void AddText(Canvas canvas, string text, double x, double y, double width, double height, double fontSize, TextAlignment alignment)
    {
        var container = new Grid
        {
            Width = width,
            Height = height,
        };
        container.Children.Add(Label(text, fontSize, alignment));
        Canvas.SetLeft(container, x);
        Canvas.SetTop(container, y);
        canvas.Children.Add(container);
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
