using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

class Program
{
    sealed class Box { public int Value = 31; }
    static int StaticValue = 29;

    static bool Check(ref int first, int expected)
    {
        ref int before = ref Unsafe.Subtract(ref first, 1);
        ref int earlier = ref Unsafe.Add(ref first, -2);
        bool signs = Unsafe.ByteOffset(ref first, ref before) == -(nint)sizeof(int)
            && Unsafe.ByteOffset(ref before, ref first) == (nint)sizeof(int)
            && Unsafe.IsAddressLessThan(ref earlier, ref before)
            && Unsafe.IsAddressGreaterThan(ref first, ref before);
        Span<int> stored = MemoryMarshal.CreateSpan(ref before, 1);
        ref int restored = ref Unsafe.Add(ref MemoryMarshal.GetReference(stored), 1);
        ref int restoredEarlier = ref Unsafe.Add(ref earlier, 2);
        return signs && restored == expected && restoredEarlier == expected
            && Unsafe.AreSame(ref first, ref restored);
    }

    static int Main()
    {
        int[] array = { 17, 19 };
        Box box = new Box();
        int local = 23;
        bool a = Check(ref array[0], 17);
        bool b = Check(ref box.Value, 31);
        bool c = Check(ref local, 23);
        bool d = Check(ref StaticValue, 29);
        Console.WriteLine($"{a} {b} {c} {d}");
        return a && b && c && d ? 42 : 1;
    }
}
