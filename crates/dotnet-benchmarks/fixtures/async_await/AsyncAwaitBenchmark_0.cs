using System.Threading.Tasks;

public static class Program
{
    private const int Iterations = 128;
    private const int ExpectedResult = 16512;

    public static int Main() => Run().GetAwaiter().GetResult() == ExpectedResult ? 0 : 1;

    private static async Task<int> Run()
    {
        var result = 0;
        for (var i = 0; i < Iterations; i++)
        {
            var completion = new TaskCompletionSource<int>();
            var pending = AwaitResult(completion.Task);
            if (pending.IsCompleted)
            {
                return -1;
            }

            // Complete the source on this thread to exercise an actual await suspension
            // without relying on a worker thread, timer, or scheduler.
            completion.SetResult(i + 1);
            result += await pending;
            result += await new ValueTask<int>(i + 1);
        }

        return result;
    }

    private static async Task<int> AwaitResult(Task<int> task) => await task;
}
