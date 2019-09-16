#!/usr/bin/env ruby

require 'pry'
require 'time'
require 'rmagick'
require 'gruff'
require 'lz4-ruby'

if ARGV.length != 1
  puts("Usage: create_gif.rb pid")
  exit
end
pid = ARGV[0]

PAGE_SIZE = 4096
# always bits
SWAPPED_BIT = 62
PRESENT_BIT = 63
ACTIVE_BIT = 58
# in-guest tracking only bits
LRU_BIT = 5

Dir.chdir("/tmp/wss/#{pid}")
timestamps = Dir.glob("20*").sort
puts("Using timestamps #{timestamps}")
virtual_addresses = Dir.glob("#{timestamps[-2]}/0x*").map{|f| f.split("/").last}.sort_by{|addr| addr.to_i(16)}
first_addr = virtual_addresses.first.to_i(16)
first_pfn = first_addr / PAGE_SIZE
last_file_pages = File.size("#{timestamps[-2]}/#{virtual_addresses.last}") / 8 # ought to be multiple of 8...
last_pfn = (virtual_addresses.last.to_i(16) / PAGE_SIZE) + last_file_pages

# This will get messy if we add in page content hashes...
image_size = Math.sqrt(last_pfn - first_pfn).ceil()
puts("Using image size #{image_size}")

label_border = (image_size / 30.0).ceil()

gif = Magick::ImageList.new

active_pages_arr = Array.new
zero_pages_arr = Array.new
mapped_pages_arr = Array.new

pixels = Array.new(3 * image_size * image_size, 0)
(0...timestamps.size).each do |timestamp_idx|
  total_pages = 0
  active_pages = 0
  zero_pages = 0
  mapped_pages = 0
  virtual_addresses.each do |virtual_address|
    puts("#{timestamp_idx} / #{timestamps.size}")
    file = File.open("#{timestamps[timestamp_idx]}/#{virtual_address}", "rb")
    puts("Opened #{timestamps[timestamp_idx]}/#{virtual_address}")
    pageflags = file.read.unpack("Q*")
    page_idx = ((virtual_address.to_i(16) / PAGE_SIZE) - first_pfn).to_i
    pageflags.each do |byte|
      total_pages += 1
      active = (byte & (1 << ACTIVE_BIT) == 0) ? false : true
      zero_mask = 1 << 57
      zero_add = ((byte & zero_mask) == 0) ? 0 : 0x80
      present = (byte & (1 << PRESENT_BIT) == 0) ? false : true
      swapped = (byte & (1 << SWAPPED_BIT) == 0) ? false : true
      zero_pages += 1 if zero_add != 0

      if present
        mapped_pages += 1
        if active
          pixels[(3 * page_idx) + 0 ] = 0x7F + zero_add
          pixels[(3 * page_idx) + 1 ] = 0
          pixels[(3 * page_idx) + 2 ] = 0
          active_pages += 1
        else
          pixels[(3 * page_idx) + 0 ] = 0
          pixels[(3 * page_idx) + 1 ] = 0x7F + zero_add
          pixels[(3 * page_idx) + 2 ] = 0
        end
      elsif swapped
          pixels[(3 * page_idx) + 0 ] = 0x60
          pixels[(3 * page_idx) + 1 ] = 0x60
          pixels[(3 * page_idx) + 2 ] = 0x60
      else
        pixels[(3 * page_idx) + 0 ] = 0
        pixels[(3 * page_idx) + 1 ] = 0
        pixels[(3 * page_idx) + 2 ] = 0
      end
      page_idx += 1
    end
  end

  active_pages_arr << active_pages / total_pages.to_f
  zero_pages_arr << zero_pages / total_pages.to_f
  mapped_pages_arr << mapped_pages / total_pages.to_f

  image = Magick::Image.new(image_size, image_size) { self.background_color = "black" }
  puts("Image size: #{image_size}")
  puts("Pixel size: #{pixels.length}")
  image.import_pixels(0, 0, image_size, image_size, "RGB", pixels.pack("C*"))
  image = image.extent(image_size, image_size + label_border)
  desc_txt = Magick::Draw.new
  image.annotate(desc_txt, 0, 0, 0, 0, "SPECjbb 1 core, 3100 MiB RAM, 2500 MiB heap") {
    desc_txt.pointsize = label_border * 0.9
    desc_txt.fill = 'red'
    desc_txt.gravity = Magick::SouthWestGravity
    desc_txt.font_weight = Magick::BoldWeight
  }
  frame_txt = Magick::Draw.new
  delta_t = Time.parse(timestamps[timestamp_idx]) - Time.parse(timestamps.first)
  image.annotate(frame_txt, 0, 0, 0, 0, Time.at(delta_t).utc.strftime("%H:%M:%S")) {
    frame_txt.pointsize = label_border * 0.9
    frame_txt.fill = 'white'
    frame_txt.gravity = Magick::SouthEastGravity
    frame_txt.font_weight = Magick::BoldWeight
  }
  #image.colorspace = Magick::GRAYColorspace
  gif << image
  image.write("img/#{timestamp_idx.to_s.rjust(3, "0")}.png")
end
gif.delay = 100
gif.write("img/gif.gif")

graph = Gruff::Line.new
graph.data("mapped", mapped_pages_arr)
graph.data("zeros", zero_pages_arr)
graph.data("active", active_pages_arr)
graph.write("img/graph.png")
