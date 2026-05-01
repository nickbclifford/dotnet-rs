using System;
using System.Collections.Generic;

public static class Program
{
    private static int _finallyAccumulator;

    private static IEnumerable<int> StressSequence()
    {
        try
        {
            for (int i = 1; i <= 5; i++)
            {
                int emitted;
                try
                {
                    if (i == 3)
                    {
                        throw new ApplicationException();
                    }

                    emitted = i * 10;
                }
                catch (ApplicationException)
                {
                    emitted = 7;
                }
                finally
                {
                    _finallyAccumulator += i;
                }

                yield return emitted;
            }
        }
        finally
        {
            _finallyAccumulator += 100;
        }
    }

    public static int Main()
    {
        int sum = 0;
        int seen = 0;
        bool caught = false;

        try
        {
            foreach (int value in StressSequence())
            {
                sum += value;
                seen++;

                if (seen == 4)
                {
                    throw new InvalidOperationException("consumer");
                }
            }
        }
        catch (InvalidOperationException ex)
        {
            caught = ex.Message == "consumer";
        }

        return caught && seen == 4 && sum == 77 && _finallyAccumulator == 110 ? 42 : 1;
    }
}
