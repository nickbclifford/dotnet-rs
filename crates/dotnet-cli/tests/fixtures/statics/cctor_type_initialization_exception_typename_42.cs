using System;

public class Broken
{
    public static int Value = Init();
    static int Init() { throw new InvalidOperationException("boom"); }
}

public class Program
{
    public static int Main()
    {
        try
        {
            int v = Broken.Value;
            return 1;
        }
        catch (TypeInitializationException e)
        {
            if (e.TypeName != "Broken") return 2;
            if (!(e.InnerException is InvalidOperationException)) return 3;
            return 42;
        }
    }
}
