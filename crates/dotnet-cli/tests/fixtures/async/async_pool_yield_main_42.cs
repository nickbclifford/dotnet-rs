using System.Threading;
using System.Threading.Tasks;

public static class Program
{
    public static async Task<int> Main()
    {
        await Task.Yield();
        return Thread.CurrentThread.IsThreadPoolThread && Thread.CurrentThread.IsBackground ? 42 : 1;
    }
}
