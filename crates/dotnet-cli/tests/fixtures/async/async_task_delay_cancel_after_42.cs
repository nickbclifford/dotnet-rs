using System;
using System.Threading;
using System.Threading.Tasks;

public static class Program
{
    public static async Task<int> Main()
    {
        using var cancellation = new CancellationTokenSource();
        cancellation.CancelAfter(50);
        try
        {
            await Task.Delay(Timeout.Infinite, cancellation.Token);
            return 1;
        }
        catch (TaskCanceledException exception)
        {
            return cancellation.IsCancellationRequested
                && exception.CancellationToken == cancellation.Token
                ? 42
                : 2;
        }
    }
}
