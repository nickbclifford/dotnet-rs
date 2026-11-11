using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        var source = new TaskCompletionSource<int>();
        return !source.Task.Wait(50) && !source.Task.IsCompleted ? 42 : 1;
    }
}
