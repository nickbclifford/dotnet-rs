using System;
using System.Collections.Generic;
using System.Numerics;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Runtime.Intrinsics;

public class Program {
    public class Widget { public int Value = 17; }
    public enum Colour { None = 0, Red = 7, Blue = 9 }
    public struct Pair { public int Left; public long Right; }
    public class Box<T> { public int Value = 23; }
    public class Caller<T> {
        public static Box<int> Create<U>() { return Activator.CreateInstance<Box<int>>(); }
    }

    static T Sum<T>(T a, T b) where T : INumber<T> {
        T result = T.Zero;
        result = result + a;
        result = result + b;
        return result > T.Zero ? ++result : result;
    }

    static bool Run<A, B>() {
        var widget = Activator.CreateInstance<Widget>();
        int number = Activator.CreateInstance<int>();
        long large = Activator.CreateInstance<long>();
        Colour colour = Activator.CreateInstance<Colour>();
        Pair pair = Activator.CreateInstance<Pair>();
        Box<int> box = Caller<string>.Create<long>();
        var data = new int[] { 11, 22, 33 };
        Span<int> span = new Span<int>(data);
        var sliced = span.Slice(1);
        ReadOnlySpan<int> readOnly = sliced;
        ref int first = ref MemoryMarshal.GetArrayDataReference(data);
        ref int second = ref Unsafe.AddByteOffset(ref first, (IntPtr)4);
        Array bytes = new byte[] { 12, 34 };
        ref byte rawByte = ref MemoryMarshal.GetArrayDataReference(bytes);
        Unsafe.Add(ref rawByte, 1) = 56;
        Array integers = new int[] { 123, 456 };
        ref byte rawInt = ref MemoryMarshal.GetArrayDataReference(integers);
        ref int firstInt = ref Unsafe.As<byte, int>(ref rawInt);
        firstInt = 789;
        bool nullThrows = false;
        try { MemoryMarshal.GetArrayDataReference((Array)null!); }
        catch (NullReferenceException) { nullThrows = true; }
        bool isNull = Unsafe.IsNullRef(ref Unsafe.NullRef<int>());
        string formatted = $"{Colour.Blue}";
        Console.WriteLine($"{widget.Value}:{number}:{large}:{(int)colour}:{pair.Left}:{pair.Right}:{box.Value}:{sliced[0]}:{readOnly[1]}:{second}:{isNull}");
        Console.WriteLine($"{EqualityComparer<int>.Default.Equals(3, 3)}:{Comparer<int>.Default.Compare(2, 3) < 0}:{Vector128<int>.IsSupported}:{Vector128<decimal>.IsSupported}");
        Console.WriteLine($"{Sum<int>(2, 3)}:{formatted}");
        Console.WriteLine($"{((byte[])bytes)[1]}:{((int[])integers)[0]}:{nullThrows}");
        return widget.Value == 17 && number == 0 && large == 0 && colour == Colour.None
            && pair.Left == 0 && pair.Right == 0 && box.Value == 23 && second == 22 && formatted == "Blue"
            && ((byte[])bytes)[1] == 56 && ((int[])integers)[0] == 789 && nullThrows;
    }

    public static int Main() { return Run<string, long>() ? 42 : 1; }
}
