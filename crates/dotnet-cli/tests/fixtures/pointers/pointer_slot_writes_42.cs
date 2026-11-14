using System;
using System.Reflection;

public unsafe class Fields
{
    public int* Instance;
}

public unsafe struct Pair
{
    public int* First;
    public void* Second;
}

public static unsafe class Program
{
    static void Store<T>(ref T destination, T value) { destination = value; }

    public static int Main()
    {
        int value = 123;
        int* p = &value;
        var fields = new Fields();
        FieldInfo instance = typeof(Fields).GetField("Instance")!;
        object boxed = Pointer.Box(p, typeof(int*));
        instance.SetValue(fields, boxed);
        if (fields.Instance != p || Pointer.Unbox(instance.GetValue(fields)!) != p) return 1;
        int*[] array = new int*[1];
        array[0] = p;
        if (array[0] != p) return 3;
        Pair pair = default;
        Store(ref pair, new Pair { First = p, Second = p });
        if (pair.First != p || pair.Second != p) return 4;
        object signed = (IntPtr)p;
        object unsigned = (UIntPtr)p;
        if ((int*)(IntPtr)signed != p || (int*)(UIntPtr)unsigned != p) return 5;
        return 42;
    }
}
