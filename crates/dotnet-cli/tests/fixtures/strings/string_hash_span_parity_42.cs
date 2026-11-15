using System;
using System.Collections.Generic;

// A string and a span with the same characters must hash alike, or Dictionary's span lookups
// (GetAlternateLookup<ReadOnlySpan<char>>, which System.Text.Json uses for property names) miss.
// Two VM bugs broke this: strings had no NUL terminator, so String.GetNonRandomizedHashCode read
// past an odd-length string's characters, and GetHashCodeOrdinalIgnoreCase was a VM intrinsic
// with its own hash instead of CoreLib's Marvin.
public static class Program
{
    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }

    public static int Main()
    {
        string[] keys = { "A", "Id", "Zip", "City", "Score", "Name12", "Length7", "EightChr", "NineChars" };
        int id = 0;
        foreach (var comparer in new StringComparer[] { StringComparer.Ordinal, StringComparer.OrdinalIgnoreCase })
        {
            var map = new Dictionary<string, int>(comparer);
            for (int i = 0; i < keys.Length; i++) map[keys[i]] = i;
            var lookup = map.GetAlternateLookup<ReadOnlySpan<char>>();
            for (int i = 0; i < keys.Length; i++)
            {
                // Copy into a larger buffer so the span is not backed by the string itself.
                Span<char> buffer = stackalloc char[16];
                buffer.Fill('#');
                keys[i].AsSpan().CopyTo(buffer);
                Check(++id, lookup.TryGetValue(buffer[..keys[i].Length], out int found) && found == i);
            }
        }

        foreach (var key in keys)
        {
            Check(++id, key.GetHashCode() == string.GetHashCode(key.AsSpan()));
            Check(++id, StringComparer.OrdinalIgnoreCase.GetHashCode(key)
                == string.GetHashCode(key.AsSpan(), StringComparison.OrdinalIgnoreCase));
            Check(++id, StringComparer.OrdinalIgnoreCase.GetHashCode(key)
                == StringComparer.OrdinalIgnoreCase.GetHashCode(key.ToUpperInvariant()));
        }

        Console.WriteLine(failures == 0 ? "ok" : "failures: " + failures);
        return failures == 0 ? 42 : 1;
    }
}
