using System.Text.Json.Serialization;

namespace KeymapOverlay;

/// <summary>Describes one platform-neutral keyboard layer display model.</summary>
internal sealed class OverlayModel
{
    [JsonPropertyName("version")]
    public int Version { get; init; }

    [JsonPropertyName("layer")]
    public byte Layer { get; init; }

    [JsonPropertyName("width")]
    public double Width { get; init; }

    [JsonPropertyName("height")]
    public double Height { get; init; }

    [JsonPropertyName("header_font_size")]
    public double HeaderFontSize { get; init; }

    [JsonPropertyName("key_font_size")]
    public double KeyFontSize { get; init; }

    [JsonPropertyName("encoder_font_size")]
    public double EncoderFontSize { get; init; }

    [JsonPropertyName("keys")]
    public required List<DisplayKey> Keys { get; init; }

    [JsonPropertyName("encoders")]
    public required List<DisplayEncoder> Encoders { get; init; }
}

/// <summary>Describes one rendered key in a layer display model.</summary>
internal sealed class DisplayKey
{
    [JsonPropertyName("x")]
    public double X { get; init; }

    [JsonPropertyName("y")]
    public double Y { get; init; }

    [JsonPropertyName("width")]
    public double Width { get; init; }

    [JsonPropertyName("height")]
    public double Height { get; init; }

    [JsonPropertyName("label")]
    public required List<string> Label { get; init; }

    [JsonPropertyName("held")]
    public bool Held { get; init; }
}

/// <summary>Describes one rendered encoder in a layer display model.</summary>
internal sealed class DisplayEncoder
{
    [JsonPropertyName("x")]
    public double X { get; init; }

    [JsonPropertyName("y")]
    public double Y { get; init; }

    [JsonPropertyName("size")]
    public double Size { get; init; }

    [JsonPropertyName("counter_clockwise")]
    public required List<string> CounterClockwise { get; init; }

    [JsonPropertyName("clockwise")]
    public required List<string> Clockwise { get; init; }

    [JsonPropertyName("press")]
    public required string Press { get; init; }

    [JsonPropertyName("held")]
    public bool Held { get; init; }
}
