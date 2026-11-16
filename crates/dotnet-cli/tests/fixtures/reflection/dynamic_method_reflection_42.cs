using System;
using System.Reflection;
using System.Reflection.Emit;

class Secret
{
    private static int Value = 42;
    private static int Read() => Value;
}

class Program
{
    static void Access(string label, DynamicMethod method)
    {
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Call, typeof(Secret).GetMethod("Read", BindingFlags.NonPublic | BindingFlags.Static));
        il.Emit(OpCodes.Ret);
        Console.WriteLine(label + ":" + ((Func<int>)method.CreateDelegate(typeof(Func<int>)))());
    }

    static void AccessField(string label, DynamicMethod method)
    {
        var il = method.GetILGenerator(128);
        il.Emit(OpCodes.Ldsfld, typeof(Secret).GetField("Value", BindingFlags.NonPublic | BindingFlags.Static));
        il.Emit(OpCodes.Ret);
        Console.WriteLine(label + ":" + ((Func<int>)method.CreateDelegate(typeof(Func<int>)))());
    }

    static int Main()
    {
        var module = typeof(Program).Module;
        Console.WriteLine("same module:" + (module == typeof(Secret).Module));
        Console.WriteLine("assembly:" + (module.Assembly == typeof(Program).Assembly));
        Console.WriteLine("different module:" + (module != typeof(string).Module));
        var owner = new DynamicMethod("owner", typeof(int), new[] { typeof(int), typeof(string) }, typeof(Secret), false);
        var hosted = new DynamicMethod("module", typeof(int), Type.EmptyTypes, module, false);
        var anonymous = new DynamicMethod("anonymous", typeof(int), Type.EmptyTypes);
        Console.WriteLine("owner module:" + (owner.Module == module));
        Console.WriteLine("hosted module:" + (hosted.Module == module));
        Console.WriteLine("anonymous module:" + (anonymous.Module == new DynamicMethod("other", null, null).Module));
        Console.WriteLine("anonymous distinct:" + (anonymous.Module != module));
        Access("anonymous access", anonymous);
        Access("module access", hosted);
        Access("owner access", new DynamicMethod("private", typeof(int), Type.EmptyTypes, typeof(string), false));
        AccessField("private field", new DynamicMethod("field", typeof(int), Type.EmptyTypes, module, false));
        AccessField("skip field", new DynamicMethod("field", typeof(int), Type.EmptyTypes, module, true));
        Console.WriteLine("parameter count:" + owner.GetParameters().Length);
        Console.WriteLine("define return:" + (owner.DefineParameter(0, ParameterAttributes.Out, "return") == null));
        Console.WriteLine("define parameter:" + (owner.DefineParameter(1, ParameterAttributes.In, "value") == null));
        var parameters = owner.GetParameters();
        var parameterCopy = owner.GetParameters();
        parameterCopy[0] = null;
        Console.WriteLine("parameter array copied:" + (owner.GetParameters()[0] != null));
        Console.WriteLine("parameter:" + parameters[0].Name + ":" + parameters[0].Attributes + ":" + parameters[0].Position + ":" + (parameters[0].ParameterType == typeof(int)) + ":" + (parameters[0].Member == owner));
        Console.WriteLine("parameter default:" + parameters[0].HasDefaultValue + ":" + (parameters[0].DefaultValue == null));
        try { _ = parameters[0].GetCustomAttributesData(); } catch (Exception e) { Console.WriteLine("parameter attributes:" + e.GetType().Name); }
        Console.WriteLine("return:" + owner.ReturnParameter.Position + ":" + (owner.ReturnParameter.ParameterType == typeof(int)) + ":" + (owner.ReturnParameter.Member == owner));
        Console.WriteLine("return attributes:" + owner.ReturnTypeCustomAttributes.GetCustomAttributes(false).Length);
        try { owner.DefineParameter(3, 0, "bad"); } catch (Exception e) { Console.WriteLine("position:" + e.GetType().Name); }
        var ownerIl = owner.GetILGenerator();
        ownerIl.Emit(OpCodes.Ldarg_0);
        ownerIl.Emit(OpCodes.Ret);
        _ = owner.CreateDelegate(typeof(Func<int, string, int>));
        Console.WriteLine("parameter after compile:" + owner.GetParameters()[0].Name);
        try { _ = owner.MethodHandle; } catch (Exception e) { Console.WriteLine("handle:" + e.GetType().Name); }
        return 42;
    }
}
