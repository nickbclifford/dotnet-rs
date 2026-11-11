using System;
using System.Runtime.CompilerServices;

public static class Program
{
    public static int Main()
    {
        string text = new string(new[] { 'a', 'b', 'c' });
        ReadOnlySpan<char> original = text.AsSpan();
        ReadOnlySpan<char> copy = Unsafe.BitCast<ReadOnlySpan<char>, ReadOnlySpan<char>>(original);
        GC.Collect();
        return copy.Length == 3 && copy[1] == 'b' && original[2] == 'c' ? 42 : 1;
    }
}
