using System;
using System.Threading.Tasks;

public static class Program
{
    private static async Task<int> ComputeAsync()
    {
        await Task.FromException(new ApplicationException("faulted-task"));
        return 0;
    }

    public static int Main()
    {
        try
        {
            _ = ComputeAsync().Result;
            return 1;
        }
        catch (ApplicationException ex)
        {
            return ex.Message == "faulted-task" ? 42 : 2;
        }
    }
}
