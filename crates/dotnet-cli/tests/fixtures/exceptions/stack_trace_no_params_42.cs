using System;
using System.Runtime.CompilerServices;

public class Program {
    public static int Main() {
        try {
            MethodNoParams();
        } catch (InvalidOperationException error) {
            string trace = error.StackTrace;
            if (error.GetType() == typeof(InvalidOperationException)
                && trace != null
                && trace.Contains("Program.MethodNoParams")
                && trace.Contains("Program.Main")) {
                return 42;
            }
        }
        return 1;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    public static void MethodNoParams() {
        throw new InvalidOperationException("no-parameter trace");
    }
}
