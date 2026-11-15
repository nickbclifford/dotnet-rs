using System;

struct Item
{
    public object Ref;
    public int Number;
    public Item(object value, int number) { Ref = value; Number = number; }
}

class Program
{
    static int Main()
    {
        int[] a = { 0, 1, 2, 3, 4, 5 };
        new Span<int>(a, 0, 5).CopyTo(new Span<int>(a, 1, 5));
        if (string.Join(",", a) != "0,0,1,2,3,4") return 1;
        new Span<int>(a, 1, 5).CopyTo(new Span<int>(a, 0, 5));
        if (string.Join(",", a) != "0,1,2,3,4,4") return 2;
        new Span<int>(a).CopyTo(new Span<int>(a));
        int[] b = new int[6];
        if (!new ReadOnlySpan<int>(a).TryCopyTo(b) || b[3] != 3) return 3;
        if (new Span<int>(a).TryCopyTo(new Span<int>(b, 0, 2)) || b[1] != 1) return 4;
        if (new ReadOnlySpan<int>(a).TryCopyTo(new Span<int>(b, 0, 2)) || b[1] != 1) return 5;
        try { new Span<int>(a).CopyTo(new Span<int>(b, 0, 2)); return 6; } catch (ArgumentException) { }
        try { new ReadOnlySpan<int>(a).CopyTo(new Span<int>(b, 0, 2)); return 7; } catch (ArgumentException) { }
        if (!default(Span<int>).TryCopyTo(default) || !default(ReadOnlySpan<int>).TryCopyTo(default)) return 8;
        default(Span<int>).CopyTo(default);
        default(ReadOnlySpan<int>).CopyTo(default);

        object x = new object(), y = new object(), z = new object();
        object[] refs = { x, y, z, x };
        new Span<object>(refs, 0, 3).CopyTo(new Span<object>(refs, 1, 3));
        if (refs[1] != x || refs[2] != y || refs[3] != z) return 9;
        new ReadOnlySpan<object>(refs, 1, 3).CopyTo(new Span<object>(refs, 0, 3));
        if (refs[0] != x || refs[1] != y || refs[2] != z) return 10;
        if (new ReadOnlySpan<object>(refs).TryCopyTo(new Span<object>(refs, 0, 2)) || refs[1] != y) return 20;
        object[] other = new object[4];
        if (!new Span<object>(refs).TryCopyTo(other) || other[2] != z) return 11;

        Item[] items = { new Item(x, 1), new Item(y, 2), new Item(z, 3), new Item(x, 4) };
        if (!new ReadOnlySpan<Item>(items, 0, 3).TryCopyTo(new Span<Item>(items, 1, 3))) return 12;
        if (items[2].Ref != y || items[2].Number != 2 || items[3].Ref != z) return 13;
        if (!new Span<Item>(items, 1, 3).TryCopyTo(new Span<Item>(items, 0, 3))) return 14;
        if (items[1].Ref != y || items[1].Number != 2 || items[2].Ref != z) return 15;

        byte[] bytes = new byte[200000];
        for (int i = 0; i < bytes.Length; i++) bytes[i] = (byte)(i % 251);
        new Span<byte>(bytes, 0, 150000).CopyTo(new Span<byte>(bytes, 10000, 150000));
        for (int i = 0; i < 150000; i++) if (bytes[i + 10000] != (byte)(i % 251)) return 16;
        new ReadOnlySpan<byte>(bytes, 10000, 150000).CopyTo(new Span<byte>(bytes, 0, 150000));
        for (int i = 0; i < 150000; i++) if (bytes[i] != (byte)(i % 251)) return 17;
        // Exercise the chunked path in both directions, with values straddling chunk boundaries.
        byte[] large = new byte[2300000];
        int[] points = { 0, 9999, 1048575, 1048576, 2097151, 2199999 };
        for (int i = 0; i < points.Length; i++) large[points[i]] = (byte)(i + 1);
        new Span<byte>(large, 0, 2200000).CopyTo(new Span<byte>(large, 10000, 2200000));
        for (int i = 0; i < points.Length; i++) if (large[points[i] + 10000] != i + 1) return 18;
        new ReadOnlySpan<byte>(large, 10000, 2200000).CopyTo(new Span<byte>(large, 0, 2200000));
        for (int i = 0; i < points.Length; i++) if (large[points[i]] != i + 1) return 19;
        int[] wide = new int[600010];
        wide[0] = 123; wide[262143] = 456; wide[599999] = 789;
        new Span<int>(wide, 0, 600000).CopyTo(new Span<int>(wide, 10, 600000));
        if (wide[10] != 123 || wide[262153] != 456 || wide[600009] != 789) return 21;
        new ReadOnlySpan<int>(wide, 10, 600000).CopyTo(new Span<int>(wide, 0, 600000));
        if (wide[0] != 123 || wide[262143] != 456 || wide[599999] != 789) return 22;
        Console.WriteLine("span copies ok");
        return 42;
    }
}
