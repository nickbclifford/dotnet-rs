using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        var tasks = new[] { Task.Run(() => 20), Task.Run(() => 22) };
        var results = Task.WhenAll(tasks).Result;
        return results[0] + results[1];
    }
}
