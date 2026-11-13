using System.Runtime.CompilerServices;

// dotnet-rs reports interpreter semantics: dynamic code is supported (EF Core requires it) but not
// compiled (the VM has no Reflection.Emit). The CLR reports both true, so this fixture is not
// compared against dotnet.
public static class Program
{
    public static int Main()
    {
        if (!RuntimeFeature.IsDynamicCodeSupported)
        {
            return 1;
        }

        if (RuntimeFeature.IsDynamicCodeCompiled)
        {
            return 2;
        }

        if (!RuntimeFeature.IsSupported("IsDynamicCodeSupported")
            || RuntimeFeature.IsSupported("IsDynamicCodeCompiled"))
        {
            return 3;
        }

        return 42;
    }
}
