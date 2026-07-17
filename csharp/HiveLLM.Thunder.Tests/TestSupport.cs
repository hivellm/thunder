using System.Globalization;

namespace HiveLLM.Thunder.Tests;

/// <summary>Shared helpers: repo-relative paths and hex byte formatting.</summary>
internal static class TestSupport
{
    /// <summary>
    /// The repo's <c>conformance/</c> directory, found by walking up from
    /// the test assembly (bin/Debug/net8.0 → repo root).
    /// </summary>
    internal static string ConformanceDir { get; } = FindConformanceDir();

    private static string FindConformanceDir()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            var candidate = Path.Combine(dir.FullName, "conformance", "vectors");
            if (Directory.Exists(candidate))
            {
                return Path.Combine(dir.FullName, "conformance");
            }

            dir = dir.Parent;
        }

        throw new DirectoryNotFoundException(
            $"conformance/ not found walking up from {AppContext.BaseDirectory}");
    }

    /// <summary>Parse space-separated hex ("08 00 00 00 …") into bytes.</summary>
    internal static byte[] ParseHex(string hex) =>
        hex.Split(' ', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .Select(b => byte.Parse(b, NumberStyles.HexNumber, CultureInfo.InvariantCulture))
            .ToArray();

    /// <summary>Format bytes as space-separated lowercase hex for readable diffs.</summary>
    internal static string ToHex(ReadOnlySpan<byte> bytes)
    {
        var parts = new string[bytes.Length];
        for (var i = 0; i < bytes.Length; i++)
        {
            parts[i] = bytes[i].ToString("x2", CultureInfo.InvariantCulture);
        }

        return string.Join(' ', parts);
    }
}
