using System.Text.Json.Serialization;

namespace KeymapOverlay;

/// <summary>Describes one platform-neutral keyboard layer display model.</summary>
internal sealed class OverlayModel
{
    [JsonPropertyName("version")]
    public int Version { get; set; }

    [JsonPropertyName("layer")]
    public byte Layer { get; set; }

    [JsonPropertyName("width")]
    public double Width { get; set; }

    [JsonPropertyName("height")]
    public double Height { get; set; }

    [JsonPropertyName("header_font_size")]
    public double HeaderFontSize { get; set; }

    [JsonPropertyName("key_font_size")]
    public double KeyFontSize { get; set; }

    [JsonPropertyName("encoder_font_size")]
    public double EncoderFontSize { get; set; }

    [JsonPropertyName("keys")]
    public required List<DisplayKey> Keys { get; set; }

    [JsonPropertyName("encoders")]
    public required List<DisplayEncoder> Encoders { get; set; }
}

/// <summary>Describes one rendered key in a layer display model.</summary>
internal sealed class DisplayKey
{
    [JsonPropertyName("x")]
    public double X { get; set; }

    [JsonPropertyName("y")]
    public double Y { get; set; }

    [JsonPropertyName("width")]
    public double Width { get; set; }

    [JsonPropertyName("height")]
    public double Height { get; set; }

    [JsonPropertyName("label")]
    public required List<string> Label { get; set; }

    [JsonPropertyName("held")]
    public bool Held { get; set; }

    [JsonPropertyName("transparent")]
    public bool Transparent { get; set; }

    [JsonPropertyName("momentary_layer")]
    public byte? MomentaryLayer { get; set; }
}

/// <summary>Describes one rendered encoder in a layer display model.</summary>
internal sealed class DisplayEncoder
{
    [JsonPropertyName("x")]
    public double X { get; set; }

    [JsonPropertyName("y")]
    public double Y { get; set; }

    [JsonPropertyName("size")]
    public double Size { get; set; }

    [JsonPropertyName("counter_clockwise")]
    public required List<string> CounterClockwise { get; set; }

    [JsonPropertyName("clockwise")]
    public required List<string> Clockwise { get; set; }

    [JsonPropertyName("press")]
    public required string Press { get; set; }

    [JsonPropertyName("held")]
    public bool Held { get; set; }

    [JsonPropertyName("counter_clockwise_transparent")]
    public bool CounterClockwiseTransparent { get; set; }

    [JsonPropertyName("clockwise_transparent")]
    public bool ClockwiseTransparent { get; set; }

    [JsonPropertyName("press_transparent")]
    public bool PressTransparent { get; set; }

    [JsonPropertyName("momentary_layer")]
    public byte? MomentaryLayer { get; set; }
}
