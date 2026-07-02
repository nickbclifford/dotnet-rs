using System.Runtime.CompilerServices;

public static class Program
{
    public static int Main()
    {
        if (!RuntimeFeature.IsDynamicCodeSupported)
        {
            return 1;
        }

        if (!RuntimeFeature.IsDynamicCodeCompiled)
        {
            return 2;
        }

        return 42;
    }
}
