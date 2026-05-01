using System;
using System.Collections.Generic;

public static class Program
{
    private static int _finallyRuns;

    public static IEnumerable<int> FaultySequence()
    {
        try
        {
            yield return 40;
            throw new InvalidOperationException();
        }
        finally
        {
            _finallyRuns += 2;
        }
    }

    public static int Main()
    {
        int sum = 0;
        bool sawExpectedException = false;

        try
        {
            foreach (int value in FaultySequence())
            {
                sum += value;
            }
        }
        catch (InvalidOperationException)
        {
            sawExpectedException = true;
        }

        return sawExpectedException && sum == 40 && _finallyRuns == 2 ? 42 : 1;
    }
}
