using System;

public delegate int Mixer(int a, int b, int c, int d);

public class Program
{
    private static int trace;

    private static int First(int a, int b, int c, int d)
    {
        trace = trace * 10 + 1;
        return a + b + c + d;
    }

    private static int Second(int a, int b, int c, int d)
    {
        trace = trace * 10 + 2;
        return a * b + c + d;
    }

    private static int Third(int a, int b, int c, int d)
    {
        trace = trace * 10 + 3;
        return a * 10 + b + c + d;
    }

    public static int Main()
    {
        Mixer direct = First;
        if (direct(1, 2, 3, 4) != 10)
            return 1;

        Mixer chain = First;
        chain += Second;
        chain += Third;

        trace = 0;
        for (int i = 0; i < 3; i++)
        {
            // Exercises repeated Invoke classification and multicast continuation argument reuse.
            if (chain(i, 2, 3, 4) != i * 10 + 9)
                return 2;
        }

        return trace == 123123123 ? 42 : 3;
    }
}
