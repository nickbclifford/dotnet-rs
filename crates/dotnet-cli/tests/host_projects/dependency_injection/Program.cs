using System;
using System.Threading;
using Microsoft.Extensions.DependencyInjection;

public sealed class Dependency { public int Value => 42; }
public sealed class Service
{
    public Dependency Dependency { get; }
    public Service(Dependency dependency) => Dependency = dependency;
}

public static class Program
{
    public static int Main()
    {
        var services = new ServiceCollection();
        services.AddSingleton<Dependency>();
        services.AddTransient<Service>();
        using var provider = services.BuildServiceProvider();
        for (int i = 0; i < 2; i++)
            if (provider.GetRequiredService<Service>().Dependency.Value != 42) return 1;
        // The dynamic provider queues compilation on its second resolution.
        Thread.Sleep(100);
        int result = provider.GetRequiredService<Service>().Dependency.Value;
        Console.WriteLine(result);
        return result;
    }
}
