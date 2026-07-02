using System;
using System.Linq;
using Microsoft.EntityFrameworkCore;

public class Blog
{
    public int Id { get; set; }
    public string Title { get; set; }
}

public class BlogContext : DbContext
{
    public DbSet<Blog> Blogs { get; set; }

    protected override void OnConfiguring(DbContextOptionsBuilder o)
        => o.UseInMemoryDatabase("test");
}

public static class Program
{
    public static int Main()
    {
        using var ctx = new BlogContext();
        ctx.Blogs.Add(new Blog { Title = "Hello" });
        ctx.SaveChanges();

        var b = ctx.Blogs.First();
        Console.WriteLine(b.Title);
        return b != null ? 42 : 1;
    }
}
