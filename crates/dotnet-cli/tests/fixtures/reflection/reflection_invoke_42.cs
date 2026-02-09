using System;
using System.Reflection;

public class Calculator {
    public int Add(int a, int b) => a + b;
    public static int StaticAdd(int a, int b) => a + b;
}

public class Program {
    public static int Main() {
        Type type = typeof(Calculator);
        
        // Static method invoke
        MethodInfo staticMethod = type.GetMethod("StaticAdd");
        if (staticMethod == null) return 1;
        object result1 = staticMethod.Invoke(null, new object[] { 20, 22 });
        if ((int)result1 != 42) return 2;
        
        // Instance method invoke
        Calculator calc = new Calculator();
        MethodInfo instanceMethod = type.GetMethod("Add");
        if (instanceMethod == null) return 3;
        object result2 = instanceMethod.Invoke(calc, new object[] { 10, 32 });
        if ((int)result2 != 42) return 4;
        
        return 42;
    }
}
