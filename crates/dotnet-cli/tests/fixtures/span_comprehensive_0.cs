using System;

public struct MyStruct
{
    public int Value;
}

public class Program
{
    public unsafe static int Main()
    {
        // 1. ReadOnlySpan<byte> backed by arrays
        byte[] b1 = new byte[5];
        b1[0] = 1; b1[1] = 2; b1[2] = 3; b1[3] = 4; b1[4] = 5;
        byte[] b2 = new byte[5];
        b2[0] = 1; b2[1] = 2; b2[2] = 3; b2[3] = 4; b2[4] = 5;
        byte[] b3 = new byte[5];
        b3[0] = 1; b3[1] = 2; b3[2] = 3; b3[3] = 4; b3[4] = 6;
        
        ReadOnlySpan<byte> rb1 = b1;
        ReadOnlySpan<byte> rb2 = b2;
        ReadOnlySpan<byte> rb3 = b3;
        
        if (!rb1.SequenceEqual(rb2)) return 1;
        if (rb1.SequenceEqual(rb3)) return 2;
        
        // 2. ReadOnlySpan<int> matching and non-matching
        int[] i1 = new int[3];
        i1[0] = 100; i1[1] = 200; i1[2] = 300;
        int[] i2 = new int[3];
        i2[0] = 100; i2[1] = 200; i2[2] = 300;
        int[] i3 = new int[3];
        i3[0] = 100; i3[1] = 201; i3[2] = 300;
        
        ReadOnlySpan<int> ri1 = i1;
        ReadOnlySpan<int> ri2 = i2;
        ReadOnlySpan<int> ri3 = i3;
        
        if (!ri1.SequenceEqual(ri2)) return 3;
        if (ri1.SequenceEqual(ri3)) return 4;
        
        // 3. Span<T> from arrays and slices
        Span<int> s1 = i1;
        Span<int> s2 = s1.Slice(1, 2); // { 200, 300 }
        if (s2.Length != 2) return 5;
        if (s2[0] != 200) return 6;
        if (s2[1] != 300) return 7;
        
        // 4. Span<T> from stackalloc
        int* stackPtr = stackalloc int[3];
        stackPtr[0] = 1;
        stackPtr[1] = 2;
        stackPtr[2] = 3;
        Span<int> sStack = new Span<int>(stackPtr, 3);
        if (sStack.Length != 3) return 8;
        if (sStack[0] != 1 || sStack[1] != 2 || sStack[2] != 3) return 9;
        
        Span<int> sStack2 = new Span<int>(stackPtr, 3);
        if (!sStack.SequenceEqual(sStack2)) return 10;
        
        // 5. CopyTo
        int[] iDest = new int[3];
        Span<int> sDest = iDest;
        sStack.CopyTo(sDest);
        if (sDest[0] != 1 || sDest[1] != 2 || sDest[2] != 3) return 11;
        
        // 6. Indexer access
        s1[0] = 999;
        if (i1[0] != 999) return 12;
        
        // 7. Empty spans
        Span<int> sEmpty1 = Span<int>.Empty;
        Span<int> sEmpty2 = new Span<int>(null, 0);
        if (!sEmpty1.SequenceEqual(sEmpty2)) return 13;
        if (sEmpty1.Length != 0) return 14;

        // 8. Span from stack-local value type (Transient origin)
        MyStruct ms = new MyStruct();
        ms.Value = 123;
        Span<int> sTransient = new Span<int>(ref ms.Value);
        if (sTransient[0] != 123) return 15;

        return 0;
    }
}
