using System;
using System.Reflection.Emit;
using System.Threading;

public struct Flag { public bool Value; }

public class Program {
    public static int Main() {
        var method = new DynamicMethod("Probe", typeof(void), new[] { typeof(object) });
        var il = method.GetILGenerator();
        il.DeclareLocal(typeof(Flag));
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldloc_0);
        il.Emit(OpCodes.Ldflda, typeof(Flag).GetField(nameof(Flag.Value))!);
        il.Emit(OpCodes.Call, typeof(Monitor).GetMethod("Enter", new[] { typeof(object), typeof(bool).MakeByRefType() })!);
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Call, typeof(Monitor).GetMethod("Exit", new[] { typeof(object) })!);
        il.Emit(OpCodes.Ret);
        try {
            ((Action<object>)method.CreateDelegate(typeof(Action<object>)))(new object());
            Console.WriteLine("accepted");
            return 1;
        } catch (InvalidProgramException) {
            Console.WriteLine("rejected");
            return 42;
        }
    }
}
