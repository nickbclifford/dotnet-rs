using System;
using System.Reflection.Emit;

class Outer
{
    public class Inner { public int Value; public Inner(int value) { Value = value; } }
}
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Nested", typeof(Outer.Inner), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Newobj, typeof(Outer.Inner).GetConstructor(new[] { typeof(int) })!);
        il.Emit(OpCodes.Ret);
        var result = ((Func<Outer.Inner>)method.CreateDelegate(typeof(Func<Outer.Inner>)))().Value;
        Console.WriteLine(result);
        return result;
    }
}
