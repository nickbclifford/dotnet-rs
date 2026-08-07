using Microsoft.EntityFrameworkCore;

public sealed class Blog
{
    public int Id { get; set; }
    public string Title { get; set; } = string.Empty;
    public int Rating { get; set; }
    public bool IsPublished { get; set; }
}

public sealed class BlogContext : DbContext
{
    public DbSet<Blog> Blogs => Set<Blog>();

    protected override void OnConfiguring(DbContextOptionsBuilder options)
        => options.UseInMemoryDatabase("dotnet-rs-benchmark");
}

public static class Program
{
    public static int Main()
    {
        using var context = new BlogContext();

        for (var i = 0; i < 24; i++)
        {
            context.Blogs.Add(new Blog
            {
                Title = "blog-" + i,
                Rating = (i * 17) % 100,
                IsPublished = i % 3 != 0,
            });
        }

        var saved = context.SaveChanges();
        var checksum = 0;
        foreach (var blog in context.Blogs
            .Where(blog => blog.IsPublished && blog.Rating >= 50)
            .OrderByDescending(blog => blog.Rating)
            .Take(8))
        {
            checksum += blog.Id;
            checksum += blog.Rating;
            checksum += blog.Title.Length;
        }

        return saved == 24 && checksum > 0 ? 0 : 1;
    }
}
