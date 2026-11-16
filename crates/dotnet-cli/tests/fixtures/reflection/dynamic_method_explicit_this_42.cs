using System;
using System.Reflection;
using System.Reflection.Emit;
public class Box { public int Read() => 42; }
class Program
{
    static void Test(bool includeThis)
    {
        try
        {
            var method = new DynamicMethod("Read", typeof(int), new[] { typeof(Box) });
            var il = method.GetILGenerator();
            il.Emit(OpCodes.Ldarg_0);
            il.Emit(OpCodes.Ldftn, typeof(Box).GetMethod(nameof(Box.Read))!);
            il.EmitCalli(OpCodes.Calli, CallingConventions.Standard | CallingConventions.HasThis | CallingConventions.ExplicitThis,
                typeof(int), includeThis ? new[] { typeof(Box) } : Type.EmptyTypes, null);
            il.Emit(OpCodes.Ret);
            var action = (Func<Box, int>)method.CreateDelegate(typeof(Func<Box, int>));
            Console.WriteLine($"{includeThis}:{action(new Box())}");
        }
        catch (Exception e) { Console.WriteLine($"{includeThis}:{e.GetType().Name}"); }
    }
    static int Main() { Test(false); Test(true); return 42; }
}
