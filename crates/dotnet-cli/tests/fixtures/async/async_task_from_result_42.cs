using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        Task<int> task = Task.FromResult(42);
        return task.Result;
    }
}
