using System;
using System.Runtime.CompilerServices;

public class Program {
    public static int Main() {
        try {
            GenericMethod<int>(42);
        } catch (InvalidOperationException error) {
            string trace = error.StackTrace;
            if (error.GetType() == typeof(InvalidOperationException)
                && trace != null
                && trace.Contains("Program.GenericMethod")
                && trace.Contains("Program.Main")) {
                return 42;
            }
        }
        return 1;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    public static void GenericMethod<T>(T value) {
        if (value is null) throw new ArgumentNullException(nameof(value));
        throw new InvalidOperationException("generic trace");
    }
}
