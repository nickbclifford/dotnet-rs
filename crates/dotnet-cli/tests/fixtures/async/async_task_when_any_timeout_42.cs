using System.Threading.Tasks;

public static class Program
{
    public static async Task<int> Main()
    {
        Task<int> unfinished = new TaskCompletionSource<int>().Task;
        Task timeout = Task.Delay(50);
        Task winner = await Task.WhenAny(unfinished, timeout);
        return ReferenceEquals(winner, timeout) && !unfinished.IsCompleted ? 42 : 1;
    }
}
