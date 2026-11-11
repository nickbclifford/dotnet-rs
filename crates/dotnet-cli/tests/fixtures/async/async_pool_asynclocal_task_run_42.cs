using System.Threading;
using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        var ambient = new AsyncLocal<string>();
        ambient.Value = "parent";

        Task<bool> worker = Task.Run(() =>
        {
            bool captured = ambient.Value == "parent";
            ambient.Value = "worker";
            return captured;
        });

        bool parentUnchanged = ambient.Value == "parent";
        return worker.Result && parentUnchanged && ambient.Value == "parent" ? 42 : 1;
    }
}
