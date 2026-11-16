using System;
using System.Reflection.Emit;

public class Outer<T>
{
    public class Box<U>
    {
        public T Value;
        public Box(T value) { Value = value; }
        public V Pick<V>(V value) => value;
    }
}
class Program
{
    static int Main()
    {
        var box = typeof(Outer<int>.Box<string>);
        var method = new DynamicMethod("Members", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        var local = il.DeclareLocal(box);
        il.Emit(OpCodes.Ldc_I4, 40);
        il.Emit(OpCodes.Newobj, box.GetConstructor(new[] { typeof(int) })!);
        il.Emit(OpCodes.Stloc, local);
        il.Emit(OpCodes.Ldloc, local);
        il.Emit(OpCodes.Ldloc, local);
        il.Emit(OpCodes.Ldfld, box.GetField("Value")!);
        il.Emit(OpCodes.Ldc_I4_2);
        il.Emit(OpCodes.Add);
        il.Emit(OpCodes.Callvirt, box.GetMethod("Pick")!.MakeGenericMethod(typeof(int)));
        il.Emit(OpCodes.Ret);
        var result = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(result);
        return result;
    }
}
