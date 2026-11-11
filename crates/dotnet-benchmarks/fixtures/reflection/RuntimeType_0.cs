using System;

class Target { }

class Program
{
    static int Main()
    {
        Type expected = typeof(Target);
        int matches = 0;
        for (int i = 0; i < 30_000; i++)
        {
            if (ReferenceEquals(expected, typeof(Target))) matches++;
        }
        return matches == 30_000 ? 0 : 1;
    }
}
