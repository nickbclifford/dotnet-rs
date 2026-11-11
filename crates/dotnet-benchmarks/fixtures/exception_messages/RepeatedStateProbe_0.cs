using System;

public static class Program
{
    private static int count;

    public static void Main()
    {
        if (++count > 2) throw new InvalidOperationException("state retained");
    }
}
