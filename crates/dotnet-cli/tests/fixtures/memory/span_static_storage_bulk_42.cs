using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

// Bulk copies, fills and clears whose source or destination is static storage. A u8 literal is a
// span over RVA static data; before the fix its pointer reached Buffer.Memmove with a null address
// and the copy segfaulted (System.Text.Json's Utf8JsonWriter writes true/false/null this way).
[InlineArray(16)]
public struct Block16 { private byte _first; }

public static class Program
{
    static ReadOnlySpan<byte> TrueLiteral => "true"u8;
    static ReadOnlySpan<byte> LongLiteral => "the quick brown fox jumps over the lazy dog"u8;
    static Block16 s_block;

    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }

    public static int Main()
    {
        // Static source → heap and stack destinations.
        var array = new byte[8];
        TrueLiteral.CopyTo(array);
        Check(1, array[0] == (byte)'t' && array[3] == (byte)'e' && array[4] == 0);
        Span<byte> stack = stackalloc byte[4];
        TrueLiteral.CopyTo(stack);
        Check(2, stack.SequenceEqual("true"u8));
        Check(3, LongLiteral.ToArray().Length == 43 && LongLiteral.Slice(4, 5).SequenceEqual("quick"u8));
        Check(4, LongLiteral.IndexOf("fox"u8) == 16);

        // Static destination: fill, clear and copy into a span over a static field.
        Span<byte> block = MemoryMarshal.CreateSpan(ref Unsafe.As<Block16, byte>(ref s_block), 16);
        block.Fill(0x7F);
        Check(5, s_block[0] == 0x7F && s_block[15] == 0x7F);
        block.Slice(2, 4).Clear();
        Check(6, s_block[1] == 0x7F && s_block[2] == 0 && s_block[5] == 0 && s_block[6] == 0x7F);
        TrueLiteral.CopyTo(block.Slice(8));
        Check(7, s_block[8] == (byte)'t' && s_block[11] == (byte)'e' && s_block[12] == 0x7F);
        // Static → static, overlapping.
        block.Slice(8, 4).CopyTo(block.Slice(10));
        Check(8, s_block[10] == (byte)'t' && s_block[13] == (byte)'e');

        // Too long a destination slice is the CLR's ArgumentException, not a VM fault.
        try { LongLiteral.CopyTo(block); Check(9, false); }
        catch (ArgumentException) { Check(9, true); }

        Console.WriteLine(failures == 0 ? "ok" : "failures: " + failures);
        return failures == 0 ? 42 : 1;
    }
}
