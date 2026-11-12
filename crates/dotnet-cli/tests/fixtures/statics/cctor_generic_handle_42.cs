using System.Runtime.CompilerServices;

public class Subject<T>
{
    public static int Value;
    static Subject() { Value = ++Program.Count; }
}

public class Wrapper<T>
{
    public static int Value;
    static Wrapper() { Value = ++Program.Count; }
}

public class Program
{
    public static int Count;

    public static int Main()
    {
        // Handle first, then ordinary field access: the initializer must not run again.
        RuntimeHelpers.RunClassConstructor(typeof(Subject<int>).TypeHandle);
        RuntimeHelpers.RunClassConstructor(typeof(Subject<string>).TypeHandle);
        if (Subject<int>.Value != 1 || Subject<string>.Value != 2 || Count != 2) return 1;

        // Field access first, then handle, for fresh instantiations (nested and array arguments).
        int a = Wrapper<long>.Value;
        int b = Wrapper<int[]>.Value;
        int c = Wrapper<Subject<int>>.Value;
        RuntimeHelpers.RunClassConstructor(typeof(Wrapper<long>).TypeHandle);
        RuntimeHelpers.RunClassConstructor(typeof(Wrapper<int[]>).TypeHandle);
        RuntimeHelpers.RunClassConstructor(typeof(Wrapper<Subject<int>>).TypeHandle);
        if (a != 3 || b != 4 || c != 5 || Count != 5) return 2;
        if (Wrapper<long>.Value != 3 || Wrapper<int[]>.Value != 4 || Wrapper<Subject<int>>.Value != 5) return 3;

        // Handle first for a mixed pair.
        RuntimeHelpers.RunClassConstructor(typeof(Wrapper<string>).TypeHandle);
        RuntimeHelpers.RunClassConstructor(typeof(Wrapper<double>).TypeHandle);
        if (Wrapper<string>.Value != 6 || Wrapper<double>.Value != 7 || Count != 7) return 4;
        return 42;
    }
}
