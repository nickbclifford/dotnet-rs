using System;
using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        var task = Task.Run((Func<int>)(() => throw new InvalidOperationException("task failure")));
        try { return task.Result; }
        catch (AggregateException e)
        {
            return e.InnerException is InvalidOperationException inner && inner.Message == "task failure" ? 42 : 1;
        }
    }
}
