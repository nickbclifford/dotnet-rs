using System;
using System.Runtime.CompilerServices;

public class Program
{
    public static unsafe int Main()
    {
        // Allocation 1: 1 byte
        byte* ptr1 = stackalloc byte[1];
        if (((long)ptr1 % 8) != 0) return 1;

        // Allocation 2: 1 byte (should be aligned to next 8-byte boundary)
        byte* ptr2 = stackalloc byte[1];
        if (((long)ptr2 % 8) != 0) return 2;

        return 0;
    }
}
