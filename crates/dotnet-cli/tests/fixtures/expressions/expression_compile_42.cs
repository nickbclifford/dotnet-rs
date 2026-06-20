using System;
using System.Linq.Expressions;

public static class Program
{
    public static int Main()
    {
        Expression<Func<int, int>> f = x => x + 1;
        return (int)f.Compile()(41);
    }
}
