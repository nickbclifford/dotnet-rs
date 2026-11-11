using System.Threading.Tasks;

public static class Program
{
    public static int Main() => Task.Run(() => 40).ContinueWith(task => task.Result + 2).Result;
}
