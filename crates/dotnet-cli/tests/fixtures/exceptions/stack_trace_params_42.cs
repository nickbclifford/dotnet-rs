using System;
using System.Runtime.CompilerServices;

public class Program {
    public static int Main() {
        try {
            MethodWithParams("hello", 42);
        } catch (InvalidOperationException error) {
            string trace = error.StackTrace;
            if (error.GetType() == typeof(InvalidOperationException)
                && trace != null
                && trace.Contains("Program.MethodWithParams")
                && trace.Contains("Program.Main")
                && (trace.Contains("String value") || trace.Contains("string value"))
                && (trace.Contains("Int32 count") || trace.Contains("int count"))) {
                return 42;
            }
        }
        return 1;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    public static void MethodWithParams(string value, int count) {
        if (value != "hello" || count != 42) throw new ArgumentException();
        throw new InvalidOperationException("parameter trace");
    }
}
